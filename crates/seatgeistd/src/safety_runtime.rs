use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use libseatgeist::{PanicStopStatus, SafetyClass, current_euid};
use serde::Deserialize;

use crate::unix_time_ms;

const CONTROL_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(super) struct PanicStopState {
    path: PathBuf,
}

impl PanicStopState {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn status(&self) -> PanicStopStatus {
        PanicStopStatus {
            enabled: self.path.exists(),
            path: self.path.clone(),
        }
    }

    pub(super) fn set_enabled(&self, enabled: bool) -> Result<PanicStopStatus> {
        if enabled {
            let parent = self.path.parent().ok_or_else(|| {
                anyhow::anyhow!("panic-stop path has no parent: {}", self.path.display())
            })?;
            fs::create_dir_all(parent)
                .with_context(|| format!("create panic-stop dir {}", parent.display()))?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("set panic-stop dir permissions {}", parent.display()))?;
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&self.path)
                .with_context(|| format!("create panic-stop file {}", self.path.display()))?;
            writeln!(file, "enabled_at_unix_ms={}", unix_time_ms()?)
                .context("write panic-stop file")?;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("set panic-stop permissions {}", self.path.display()))?;
        } else {
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("remove panic-stop file {}", self.path.display())
                    });
                }
            }
        }
        Ok(self.status())
    }
}

#[derive(Debug, Clone)]
pub(super) struct ControlRateLimiter {
    limit_per_minute: Option<u32>,
    accepted: Arc<Mutex<VecDeque<Instant>>>,
}

impl ControlRateLimiter {
    pub(super) fn new(limit_per_minute: Option<u32>) -> Self {
        Self {
            limit_per_minute,
            accepted: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub(super) fn check(&self, safety_class: &SafetyClass) -> Result<()> {
        let Some(limit) = self.limit_per_minute else {
            return Ok(());
        };
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let now = Instant::now();
        let mut accepted = self
            .accepted
            .lock()
            .map_err(|_| anyhow::anyhow!("control rate-limit lock is poisoned"))?;
        while accepted
            .front()
            .is_some_and(|timestamp| now.duration_since(*timestamp) >= CONTROL_RATE_LIMIT_WINDOW)
        {
            accepted.pop_front();
        }
        if accepted.len() >= limit {
            bail!(
                "control rate limit exceeded for {:?}: {} accepted control requests in {}s; wait or adjust safety.control_rate_limit_per_minute",
                safety_class,
                limit,
                CONTROL_RATE_LIMIT_WINDOW.as_secs()
            );
        }
        accepted.push_back(now);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ApprovalStore {
    path: Option<PathBuf>,
}

impl ApprovalStore {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    pub(super) fn matching_prompt_approval(
        &self,
        safety_class: &SafetyClass,
        method: &str,
    ) -> Result<Option<String>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_approval_file_metadata(path, &metadata)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
        }

        let now = unix_time_ms()?;
        let contents = fs::read_to_string(path)
            .with_context(|| format!("read approval file {}", path.display()))?;
        for (index, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let grant: ApprovalGrant = serde_json::from_str(line).with_context(|| {
                format!(
                    "parse approval grant line {} in {}",
                    index + 1,
                    path.display()
                )
            })?;
            if grant.expires_unix_ms < now {
                continue;
            }
            if &grant.safety_class != safety_class {
                continue;
            }
            if grant.method != method && grant.method != "*" {
                continue;
            }
            return Ok(Some(grant.reason.unwrap_or_else(|| {
                format!("approval file grant for {safety_class:?}/{method}")
            })));
        }
        Ok(None)
    }
}

#[derive(Debug, Deserialize)]
struct ApprovalGrant {
    safety_class: SafetyClass,
    method: String,
    expires_unix_ms: u64,
    reason: Option<String>,
}

fn validate_approval_file_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    validate_approval_file_parent(path)?;
    if !metadata.file_type().is_file() {
        bail!("approval file must be a regular file: {}", path.display());
    }
    let uid = current_euid().context("read effective uid for approval file check")?;
    if metadata.uid() != uid {
        bail!(
            "approval file {} is owned by uid {}, expected {}",
            path.display(),
            metadata.uid(),
            uid
        );
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "approval file {} must not be readable, writable, or executable by group/other; mode is {:o}",
            path.display(),
            mode
        );
    }
    Ok(())
}

fn validate_approval_file_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("approval file has no parent: {}", path.display()))?;
    let metadata = fs::symlink_metadata(parent)
        .with_context(|| format!("stat approval file parent {}", parent.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "approval file parent must be a directory: {}",
            parent.display()
        );
    }
    let uid = current_euid().context("read effective uid for approval parent check")?;
    if metadata.uid() != uid {
        bail!(
            "approval file parent {} is owned by uid {}, expected {}",
            parent.display(),
            metadata.uid(),
            uid
        );
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        bail!(
            "approval file parent {} must not be writable by group/other; mode is {:o}",
            parent.display(),
            mode
        );
    }
    Ok(())
}
