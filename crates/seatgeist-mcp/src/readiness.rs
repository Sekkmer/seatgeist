use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde_json::Value;

const CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl SocketIdentity {
    fn read(path: &Path) -> Result<Self> {
        let metadata =
            fs::metadata(path).with_context(|| format!("stat daemon socket {}", path.display()))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        })
    }
}

#[derive(Debug, Clone)]
struct CachedReadiness {
    socket: SocketIdentity,
    stored_at: Instant,
    result: Value,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReadinessCache {
    entry: Arc<Mutex<Option<CachedReadiness>>>,
}

impl ReadinessCache {
    pub(crate) fn get(&self, socket: &Path) -> Result<Option<Value>> {
        let identity = SocketIdentity::read(socket)?;
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| anyhow::anyhow!("readiness cache lock is poisoned"))?;
        let Some(cached) = entry.as_ref() else {
            return Ok(None);
        };
        if cached.socket != identity || cached.stored_at.elapsed() > CACHE_TTL {
            *entry = None;
            return Ok(None);
        }
        Ok(Some(cached.result.clone()))
    }

    pub(crate) fn store(&self, socket: &Path, result: Value) -> Result<()> {
        let cached = CachedReadiness {
            socket: SocketIdentity::read(socket)?,
            stored_at: Instant::now(),
            result,
        };
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| anyhow::anyhow!("readiness cache lock is poisoned"))?;
        *entry = Some(cached);
        Ok(())
    }

    pub(crate) fn invalidate(&self) -> Result<()> {
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| anyhow::anyhow!("readiness cache lock is poisoned"))?;
        *entry = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixListener, path::PathBuf};

    use serde_json::json;

    use super::*;

    fn socket_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "seatgeist-readiness-cache-{label}-{}-{}.sock",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ))
    }

    #[test]
    fn cache_reuses_only_the_same_live_daemon_socket() {
        let path = socket_path("identity");
        let listener = UnixListener::bind(&path).expect("fixture socket binds");
        let cache = ReadinessCache::default();
        cache
            .store(&path, json!({"status": "ready"}))
            .expect("result caches");
        assert_eq!(
            cache.get(&path).expect("cache reads"),
            Some(json!({"status": "ready"}))
        );

        drop(listener);
        fs::remove_file(&path).expect("old socket removes");
        let replacement = UnixListener::bind(&path).expect("replacement socket binds");
        assert_eq!(cache.get(&path).expect("cache rechecks identity"), None);
        drop(replacement);
        fs::remove_file(path).ok();
    }

    #[test]
    fn explicit_invalidation_drops_cached_readiness() {
        let path = socket_path("invalidate");
        let listener = UnixListener::bind(&path).expect("fixture socket binds");
        let cache = ReadinessCache::default();
        cache
            .store(&path, json!({"status": "ready"}))
            .expect("result caches");
        cache.invalidate().expect("cache invalidates");
        assert_eq!(cache.get(&path).expect("cache reads"), None);
        drop(listener);
        fs::remove_file(path).ok();
    }
}
