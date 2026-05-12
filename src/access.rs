use std::{path::{Path, PathBuf}, sync::Arc};
use tokio::fs;

use crate::{stats, AppState};

/// 记录一次访问
pub async fn record_access(state: &Arc<AppState>, ip: &str, uuid: &str) {
    let has_uuid = is_valid_uuid(uuid);

    let target_dir: PathBuf = if has_uuid {
        state.config.users_dir.join(uuid)
    } else {
        state.config.users_dir.join("public").join(ip_to_dir(ip))
    };

    let is_new = !target_dir.exists();

    if is_new {
        if let Err(e) = fs::create_dir_all(&target_dir).await {
            tracing::warn!("create user dir failed: {}", e);
            return;
        }

        // GeoIP 查询
        let geo = state
            .geoip_cache
            .get(ip, &state.config.geoip_cache_dir, &state.geoip_notify)
            .await;

        // 写 geo.json（含原始 IP）
        let geo_file = target_dir.join("geo.json");
        if let Ok(bytes) = serde_json::to_vec(&GeoWithIp {
            ip:           ip.to_string(),
            country:      geo.country.clone(),
            country_code: geo.country_code.clone(),
            city:         geo.city.clone(),
            lat:          geo.lat,
            lon:          geo.lon,
            timezone:     geo.timezone.clone(),
        }) {
            let _ = fs::write(&geo_file, bytes).await;
        }

        // 统计新用户
        stats::record_new_user(has_uuid, &geo.country_code, &geo.country);
    } else {
        // 已有用户只计访问次数
        stats::record_access(has_uuid);
    }

    // 更新 times.txt
    update_times(&target_dir).await;

    // 检查是否需要刷盘统计
    let stats_file     = state.config.stats_file.clone();
    let flush_interval = state.config.flush_interval;
    tokio::spawn(async move {
        stats::try_flush(&stats_file, flush_interval).await;
    });
}

/// geo.json 结构（含原始 IP）
#[derive(serde::Serialize)]
struct GeoWithIp {
    ip:           String,
    country:      String,
    country_code: String,
    city:         String,
    lat:          f64,
    lon:          f64,
    timezone:     String,
}

/// IP 转换为安全的目录名（IPv6 冒号替换为下划线）
#[inline]
fn ip_to_dir(ip: &str) -> String {
    ip.replace(':', "_")
}

/// 更新访问时间记录
/// times.txt 格式：
/// 第1行：访问次数
/// 第2行：首次访问时间（Unix 时间戳）
/// 第3行：最近访问时间（Unix 时间戳）
async fn update_times(dir: &Path) {
    let file = dir.join("times.txt");
    let now  = unix_now_str();

    let (count, first) = if let Ok(content) = fs::read_to_string(&file).await {
        let mut lines = content.lines();
        let count = lines.next()
            .and_then(|l| l.parse::<u64>().ok())
            .unwrap_or(0);
        let first = lines.next().unwrap_or(&now).to_string();
        (count + 1, first)
    } else {
        (1u64, now.clone())
    };

    let content = format!("{}\n{}\n{}\n", count, first, now);
    if let Err(e) = fs::write(&file, content).await {
        tracing::warn!("write times.txt failed: {}", e);
    }
}

/// UUID v4 格式校验（零正则，零分配）
#[inline]
pub fn is_valid_uuid(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let b = s.as_bytes();
    if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| {
        matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit()
    })
}

fn unix_now_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
