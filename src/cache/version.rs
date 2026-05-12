use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::Arc};
use tokio::fs;

#[derive(Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version_name: String,
    pub data:         serde_json::Value,
    pub dir_mtime:    u64,
}

pub struct VersionCache {
    inner: DashMap<u32, Arc<VersionEntry>>,
}

impl VersionCache {
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_capacity(64),
        }
    }

    pub async fn get_or_load(
        &self,
        images_dir:        &Path,
        version_cache_dir: &Path,
        custom_id:         u32,
    ) -> Option<Arc<VersionEntry>> {
        let custom_dir = images_dir.join(custom_id.to_string());
        let dir_mtime  = get_dir_mtime(&custom_dir).await?;

        // ① 内存命中且 mtime 未变
        if let Some(e) = self.inner.get(&custom_id) {
            if e.dir_mtime >= dir_mtime {
                return Some(Arc::clone(&e));
            }
        }

        // ② 磁盘缓存命中
        let disk_path = version_cache_dir.join(format!("{}.json", custom_id));
        if let Some(entry) = load_disk_cache(&disk_path, dir_mtime).await {
            let arc = Arc::new(entry);
            self.inner.insert(custom_id, Arc::clone(&arc));
            return Some(arc);
        }

        // ③ 扫描目录重建
        let entry = scan_latest(&custom_dir, dir_mtime).await?;
        let arc   = Arc::new(entry);

        // 后台写磁盘缓存，不阻塞响应
        let arc2      = Arc::clone(&arc);
        let disk_path = disk_path.clone();
        tokio::spawn(async move {
            if let Ok(bytes) = serde_json::to_vec(&*arc2) {
                let _ = fs::write(&disk_path, bytes).await;
            }
        });

        self.inner.insert(custom_id, Arc::clone(&arc));
        Some(arc)
    }
}

async fn get_dir_mtime(path: &PathBuf) -> Option<u64> {
    fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

async fn load_disk_cache(path: &PathBuf, dir_mtime: u64) -> Option<VersionEntry> {
    let bytes = fs::read(path).await.ok()?;
    let entry: VersionEntry = serde_json::from_slice(&bytes).ok()?;
    if entry.dir_mtime >= dir_mtime {
        Some(entry)
    } else {
        None
    }
}

async fn scan_latest(custom_dir: &PathBuf, dir_mtime: u64) -> Option<VersionEntry> {
    let mut rd = fs::read_dir(custom_dir).await.ok()?;
    let mut versions: Vec<(String, serde_json::Value)> = Vec::new();

    while let Ok(Some(entry)) = rd.next_entry().await {
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let ver = entry.file_name().to_string_lossy().into_owned();
        let data_file = entry.path().join("data.json");
        if let Ok(bytes) = fs::read(&data_file).await {
            if let Ok(json) = serde_json::from_slice(&bytes) {
                versions.push((ver, json));
            }
        }
    }

    if versions.is_empty() {
        return None;
    }

    versions.sort_unstable_by(|(a, _), (b, _)| version_cmp(b, a));

    let (version_name, data) = versions.remove(0);
    Some(VersionEntry { version_name, data, dir_mtime })
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None)       => return std::cmp::Ordering::Equal,
            (Some(_), None)    => return std::cmp::Ordering::Greater,
            (None, Some(_))    => return std::cmp::Ordering::Less,
            (Some(x), Some(y)) => {
                let xa: u32 = x.parse().unwrap_or(0);
                let ya: u32 = y.parse().unwrap_or(0);
                match xa.cmp(&ya) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
        }
    }
}
