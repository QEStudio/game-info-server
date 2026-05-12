use crate::{
    cache::{geoip::GeoIpCache, version::VersionCache},
    config::Config,
};
use std::sync::atomic::AtomicU32;
use tokio::sync::Notify;

pub struct AppState {
    pub config:         Config,
    pub version_cache:  VersionCache,
    pub geoip_cache:    GeoIpCache,
    pub pending_access: AtomicU32,
    pub geoip_notify:   Notify,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            version_cache:  VersionCache::new(),
            geoip_cache:    GeoIpCache::new(),
            pending_access: AtomicU32::new(0),
            geoip_notify:   Notify::new(),
            config,
        }
    }
}