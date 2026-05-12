use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs,
    sync::Mutex,
    time::{sleep, Duration},
};
use tracing::{info, warn};

use crate::AppState;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GeoInfo {
    pub country:      String,
    pub country_code: String,
    pub city:         String,
    pub lat:          f64,
    pub lon:          f64,
    pub timezone:     String,
}

impl GeoInfo {
    pub fn local() -> Self {
        Self {
            country:      "Local".to_string(),
            country_code: "LOCAL".to_string(),
            city:         "Local Network".to_string(),
            lat:          0.0,
            lon:          0.0,
            timezone:     "UTC".to_string(),
        }
    }

    pub fn unknown() -> Self {
        Self {
            country:      "Unknown".to_string(),
            country_code: "XX".to_string(),
            city:         "Unknown".to_string(),
            lat:          0.0,
            lon:          0.0,
            timezone:     "UTC".to_string(),
        }
    }
}

/// IP 处理状态
#[derive(Clone, PartialEq)]
enum IpState {
    Queued,
    InFlight,
    Done,
}

pub struct GeoIpCache {
    /// 已完成的结果缓存
    inner: DashMap<String, Arc<GeoInfo>>,
    /// 每个 IP 的状态追踪
    state: DashMap<String, IpState>,
    /// 有序待查队列
    queue: Mutex<VecDeque<String>>,
}

impl GeoIpCache {
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_capacity(256),
            state: DashMap::with_capacity(256),
            queue: Mutex::new(VecDeque::new()),
        }
    }

    /// 查询 GeoIP，命中直接返回，否则加入异步队列返回占位值
    pub async fn get(
        &self,
        ip:              &str,
        geoip_cache_dir: &Path,
        notify:          &tokio::sync::Notify,
    ) -> Arc<GeoInfo> {
        if is_private(ip) {
            return Arc::new(GeoInfo::local());
        }

        // ① 内存命中
        if let Some(geo) = self.inner.get(ip) {
            return Arc::clone(&geo);
        }

        // ② 已在处理中，直接返回占位
        if self.state.contains_key(ip) {
            return Arc::new(GeoInfo::unknown());
        }

        // ③ 读磁盘缓存
        let cache_file = ip_cache_path(geoip_cache_dir, ip);
        if let Ok(bytes) = fs::read(&cache_file).await {
            if let Ok(geo) = serde_json::from_slice::<GeoInfo>(&bytes) {
                let arc = Arc::new(geo);
                self.inner.insert(ip.to_string(), Arc::clone(&arc));
                self.state.insert(ip.to_string(), IpState::Done);
                return arc;
            }
        }

        // ④ 首次出现，原子插入状态并入队
        let mut is_new = false;
        self.state
            .entry(ip.to_string())
            .or_insert_with(|| {
                is_new = true;
                IpState::Queued
            });

        if is_new {
            self.queue.lock().await.push_back(ip.to_string());
            notify.notify_one();
        }

        Arc::new(GeoInfo::unknown())
    }

    /// Worker 取出一批 IP 并标记为 InFlight
    pub async fn dequeue_batch(&self, max: usize) -> Vec<String> {
        let mut queue = self.queue.lock().await;
        let mut batch = Vec::with_capacity(max.min(queue.len()));

        while batch.len() < max {
            match queue.pop_front() {
                Some(ip) => {
                    if let Some(mut s) = self.state.get_mut(&ip) {
                        if *s == IpState::Queued {
                            *s = IpState::InFlight;
                            batch.push(ip);
                        }
                    }
                }
                None => break,
            }
        }

        batch
    }

    /// 查询成功，写入结果并标记 Done
    pub fn complete(&self, ip: String, geo: Arc<GeoInfo>) {
        self.inner.insert(ip.clone(), Arc::clone(&geo));
        self.state.insert(ip, IpState::Done);
    }

    /// 查询失败，移除状态允许下次重试
    pub fn fail(&self, ip: &str) {
        self.state.remove(ip);
    }

    /// 队列是否还有待处理的 IP
    pub async fn has_pending(&self) -> bool {
        !self.queue.lock().await.is_empty()
    }
}

/// GeoIP 后台 Worker（常驻）
pub async fn run_geoip_worker(state: Arc<AppState>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_max_idle_per_host(2)
        .user_agent("game-info-server/1.0")
        .build()
        .expect("reqwest client build failed");

    let max_concurrency = state.config.geoip_workers;

    loop {
        tokio::select! {
            _ = state.geoip_notify.notified() => {}
            _ = sleep(Duration::from_secs(60)) => {}
        }

        loop {
            let batch = state.geoip_cache.dequeue_batch(max_concurrency).await;
            if batch.is_empty() {
                break;
            }

            let mut handles = Vec::with_capacity(batch.len());
            for ip in batch {
                let client    = client.clone();
                let state_ref = Arc::clone(&state);

                handles.push(tokio::spawn(async move {
                    match fetch_geoip(&client, &ip).await {
                        Ok(geo) => {
                            let arc        = Arc::new(geo);
                            let cache_file = ip_cache_path(
                                &state_ref.config.geoip_cache_dir, &ip,
                            );
                            if let Ok(bytes) = serde_json::to_vec(&*arc) {
                                if let Err(e) = fs::write(&cache_file, bytes).await {
                                    warn!("geoip cache write failed {}: {}", ip, e);
                                }
                            }
                            info!("geoip resolved: {} -> {}", ip, arc.country_code);
                            state_ref.geoip_cache.complete(ip, arc);
                        }
                        Err(e) => {
                            warn!("geoip failed {}: {}", ip, e);
                            state_ref.geoip_cache.fail(&ip);
                        }
                    }
                }));
            }

            for h in handles {
                let _ = h.await;
            }

            if !state.geoip_cache.has_pending().await {
                break;
            }
        }
    }
}

async fn fetch_geoip(client: &reqwest::Client, ip: &str) -> Result<GeoInfo, String> {
    let url = format!(
        "http://ip-api.com/json/{}?fields=status,country,countryCode,city,lat,lon,timezone",
        ip
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct ApiResp {
        status:       Option<String>,
        country:      Option<String>,
        #[serde(rename = "countryCode")]
        country_code: Option<String>,
        city:         Option<String>,
        lat:          Option<f64>,
        lon:          Option<f64>,
        timezone:     Option<String>,
    }

    let data = resp.json::<ApiResp>().await.map_err(|e| e.to_string())?;

    if data.status.as_deref() != Some("success") {
        return Err("api status not success".into());
    }

    Ok(GeoInfo {
        country:      data.country.unwrap_or_default(),
        country_code: data.country_code.unwrap_or_else(|| "XX".into()),
        city:         data.city.unwrap_or_default(),
        lat:          data.lat.unwrap_or(0.0),
        lon:          data.lon.unwrap_or(0.0),
        timezone:     data.timezone.unwrap_or_else(|| "UTC".into()),
    })
}

/// 判断是否为私有/本地 IP
pub fn is_private(ip: &str) -> bool {
    matches!(ip, "unknown" | "127.0.0.1" | "::1")
        || ip.starts_with("192.168.")
        || ip.starts_with("10.")
        || ip.starts_with("172.")
        || ip.starts_with("fc")
        || ip.starts_with("fd")
}

/// IP 字符串 -> 磁盘缓存文件路径
fn ip_cache_path(dir: &Path, ip: &str) -> PathBuf {
    let filename = ip.replace(':', "_");
    dir.join(format!("{}.json", filename))
}
