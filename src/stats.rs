use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{fs, time::sleep};
use tracing::warn;

use crate::AppState;

// ── 原子计数器（热路径完全无锁）─────────────────────────────────
static PENDING_ACCESS:    AtomicU64 = AtomicU64::new(0);
static PENDING_USERS:     AtomicU64 = AtomicU64::new(0);
static PENDING_WITH_UUID: AtomicU64 = AtomicU64::new(0);
static PENDING_WITHOUT:   AtomicU64 = AtomicU64::new(0);

/// 地区增量（写频率低，用 Mutex 可接受）
static PENDING_REGIONS: Lazy<Mutex<HashMap<String, (String, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct RegionStat {
    pub country: String,
    pub count:   u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Stats {
    pub total_users:   u64,
    pub total_access:  u64,
    pub with_uuid:     u64,
    pub without_uuid:  u64,
    pub regions:       HashMap<String, RegionStat>,
    pub last_updated:  String,
}

/// 热路径：仅原子递增，不触碰磁盘
#[inline(always)]
pub fn record_access(has_uuid: bool) {
    PENDING_ACCESS.fetch_add(1, Ordering::Relaxed);
    if has_uuid {
        PENDING_WITH_UUID.fetch_add(1, Ordering::Relaxed);
    } else {
        PENDING_WITHOUT.fetch_add(1, Ordering::Relaxed);
    }
}

/// 新用户（低频路径）
pub fn record_new_user(has_uuid: bool, country_code: &str, country: &str) {
    PENDING_USERS.fetch_add(1, Ordering::Relaxed);
    if has_uuid {
        PENDING_WITH_UUID.fetch_add(1, Ordering::Relaxed);
    } else {
        PENDING_WITHOUT.fetch_add(1, Ordering::Relaxed);
    }
    if country_code != "XX" && country_code != "LOCAL" && !country_code.is_empty() {
        if let Ok(mut map) = PENDING_REGIONS.lock() {
            let entry = map
                .entry(country_code.to_string())
                .or_insert_with(|| (country.to_string(), 0));
            entry.1 += 1;
        }
    }
}

/// 达到阈值时触发刷盘（由请求路径的后台 spawn 调用）
pub async fn try_flush(stats_file: &PathBuf, threshold: u32) {
    if PENDING_ACCESS.load(Ordering::Relaxed) < threshold as u64 {
        return;
    }
    do_flush(stats_file).await;
}

/// 强制刷盘（关闭时调用）
pub async fn force_flush(state: &AppState) {
    do_flush(&state.config.stats_file).await;
}

/// 后台定时刷盘 Worker（60s 兜底）
pub async fn run_flush_worker(state: Arc<AppState>) {
    loop {
        sleep(Duration::from_secs(60)).await;
        do_flush(&state.config.stats_file).await;
    }
}

async fn do_flush(stats_file: &PathBuf) {
    // 原子交换取走增量
    let access  = PENDING_ACCESS.swap(0, Ordering::AcqRel);
    let users   = PENDING_USERS.swap(0, Ordering::AcqRel);
    let with_u  = PENDING_WITH_UUID.swap(0, Ordering::AcqRel);
    let without = PENDING_WITHOUT.swap(0, Ordering::AcqRel);

    let regions_delta: HashMap<String, (String, u64)> = {
        let mut map = PENDING_REGIONS.lock().unwrap();
        std::mem::take(&mut *map)
    };

    if access == 0 && users == 0 && regions_delta.is_empty() {
        return;
    }

    // 读取现有 stats
    let mut stats: Stats = if let Ok(b) = fs::read(stats_file).await {
        serde_json::from_slice(&b).unwrap_or_default()
    } else {
        Stats::default()
    };

    stats.total_access  += access;
    stats.total_users   += users;
    stats.with_uuid     += with_u;
    stats.without_uuid  += without;

    for (code, (country, count)) in regions_delta {
        let entry = stats.regions
            .entry(code)
            .or_insert_with(|| RegionStat { country: country.clone(), count: 0 });
        entry.count += count;
    }

    stats.last_updated = unix_now_str();

    match serde_json::to_vec_pretty(&stats) {
        Ok(bytes) => {
            if let Some(parent) = stats_file.parent() {
                let _ = fs::create_dir_all(parent).await;
            }
            if let Err(e) = fs::write(stats_file, bytes).await {
                warn!("stats write error: {}", e);
                // 写失败时把增量加回去，防止数据丢失
                PENDING_ACCESS.fetch_add(access, Ordering::Relaxed);
                PENDING_USERS.fetch_add(users, Ordering::Relaxed);
                PENDING_WITH_UUID.fetch_add(with_u, Ordering::Relaxed);
                PENDING_WITHOUT.fetch_add(without, Ordering::Relaxed);
            }
        }
        Err(e) => warn!("stats serialize error: {}", e),
    }
}

/// 当前 Unix 时间字符串
fn unix_now_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{}", secs)
}