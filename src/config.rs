use std::path::PathBuf;

pub struct Config {
    pub data_dir:          PathBuf,
    pub images_dir:        PathBuf,
    pub users_dir:         PathBuf,
    pub cache_dir:         PathBuf,
    pub stats_file:        PathBuf,
    pub version_cache_dir: PathBuf,
    pub geoip_cache_dir:   PathBuf,
    pub flush_interval:    u32,
    pub bind_addr:         String,
    pub geoip_workers:     usize,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = PathBuf::from(
            std::env::var("DATA_DIR").unwrap_or_else(|_| "../data".into()),
        );
        let cache_dir = data_dir.join("cache");

        Self {
            images_dir:        data_dir.join("images"),
            users_dir:         data_dir.join("users"),
            version_cache_dir: cache_dir.join("versions"),
            geoip_cache_dir:   cache_dir.join("geoip"),
            stats_file:        cache_dir.join("stats.json"),
            cache_dir,
            data_dir,
            flush_interval: env_u32("FLUSH_INTERVAL", 10),
            bind_addr: std::env::var("BIND_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3000".into()),
            geoip_workers: env_usize("GEOIP_WORKERS", 3),
        }
    }
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}