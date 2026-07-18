use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use libseatgeist::current_euid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FORMAT_VERSION: u32 = 1;
const MAX_TOKEN_BYTES: usize = 8192;

#[derive(Debug, Clone)]
pub(crate) struct CaptureRestoreTokenStore {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredRestoreToken {
    pub(crate) token: String,
    pub(crate) reference: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    version: u32,
    target_key: String,
    token: String,
}

impl CaptureRestoreTokenStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self, target_window_id: &str) -> Result<Option<StoredRestoreToken>> {
        let document = match self.read_document()? {
            Some(document) => document,
            None => return Ok(None),
        };
        let target_key = target_key(target_window_id)?;
        if document.target_key != target_key {
            return Ok(None);
        }
        validate_token(&document.token)?;
        Ok(Some(StoredRestoreToken {
            token: document.token,
            reference: token_reference(&target_key),
        }))
    }

    pub(crate) fn reference_for(&self, target_window_id: &str) -> Result<String> {
        Ok(token_reference(&target_key(target_window_id)?))
    }

    pub(crate) fn save(&self, target_window_id: &str, token: &str) -> Result<String> {
        validate_token(token)?;
        let target_key = target_key(target_window_id)?;
        let document = StoredDocument {
            version: FORMAT_VERSION,
            target_key: target_key.clone(),
            token: token.to_string(),
        };
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("capture restore-token path has no parent"))?;
        prepare_private_parent(parent)?;
        if path_exists_no_follow(&self.path)? {
            validate_private_file(&self.path)?;
        }

        let temporary_path = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("capture-restore"),
            Uuid::new_v4().simple()
        ));
        let write_result = (|| -> Result<()> {
            let mut temporary = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary_path)
                .with_context(|| {
                    format!(
                        "create private capture restore-token file {}",
                        temporary_path.display()
                    )
                })?;
            serde_json::to_writer(&mut temporary, &document)
                .context("serialize capture restore token")?;
            temporary.write_all(b"\n")?;
            temporary.sync_all()?;
            fs::rename(&temporary_path, &self.path).with_context(|| {
                format!(
                    "install private capture restore-token file {}",
                    self.path.display()
                )
            })?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result?;
        validate_private_file(&self.path)?;
        Ok(token_reference(&target_key))
    }

    fn read_document(&self) -> Result<Option<StoredDocument>> {
        if !path_exists_no_follow(&self.path)? {
            return Ok(None);
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("capture restore-token path has no parent"))?;
        validate_private_parent(parent)?;
        validate_private_file(&self.path)?;
        let mut content = Vec::new();
        File::open(&self.path)
            .with_context(|| format!("open capture restore-token file {}", self.path.display()))?
            .take((MAX_TOKEN_BYTES * 2) as u64)
            .read_to_end(&mut content)?;
        let document: StoredDocument =
            serde_json::from_slice(&content).context("parse capture restore-token file")?;
        if document.version != FORMAT_VERSION {
            bail!(
                "unsupported capture restore-token format version {}",
                document.version
            );
        }
        Ok(Some(document))
    }
}

fn target_key(target_window_id: &str) -> Result<String> {
    if target_window_id.trim().is_empty() {
        bail!("capture restore-token target window id must be non-empty");
    }
    Ok(hex_sha256(target_window_id.as_bytes()))
}

fn token_reference(target_key: &str) -> String {
    format!("screencast-{}", &target_key[..16])
}

fn validate_token(token: &str) -> Result<()> {
    if token.trim().is_empty() || token.len() > MAX_TOKEN_BYTES || token.contains('\0') {
        bail!("capture restore token is empty or exceeds the private storage bound");
    }
    Ok(())
}

fn prepare_private_parent(parent: &Path) -> Result<()> {
    if !parent.exists() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "create capture restore-token directory {}",
                parent.display()
            )
        })?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    validate_private_parent(parent)
}

fn validate_private_parent(parent: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("capture restore-token parent must be a real directory");
    }
    if metadata.uid() != current_euid()? {
        bail!("capture restore-token parent is not owned by the daemon user");
    }
    if metadata.mode() & 0o022 != 0 {
        bail!("capture restore-token parent is writable by group or other");
    }
    Ok(())
}

fn validate_private_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("capture restore-token path must be a regular file");
    }
    if metadata.uid() != current_euid()? {
        bail!("capture restore-token file is not owned by the daemon user");
    }
    if metadata.mode() & 0o077 != 0 {
        bail!("capture restore-token file must not be accessible by group or other");
    }
    Ok(())
}

fn path_exists_no_follow(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn temporary_store(label: &str) -> (PathBuf, CaptureRestoreTokenStore) {
        let root = std::env::temp_dir().join(format!(
            "seatgeist-capture-restore-{label}-{}",
            Uuid::new_v4().simple()
        ));
        let path = root.join("state/capture-restore.json");
        (root, CaptureRestoreTokenStore::new(path))
    }

    #[test]
    fn stores_one_private_rotating_token_without_window_identity() -> Result<()> {
        let (root, store) = temporary_store("rotation");
        let reference = store.save("kwin-window-private", "first-token")?;
        assert!(reference.starts_with("screencast-"));
        assert!(!reference.contains("private"));
        let restarted = CaptureRestoreTokenStore::new(store.path.clone());
        assert_eq!(
            restarted.load("kwin-window-private")?,
            Some(StoredRestoreToken {
                token: "first-token".to_string(),
                reference: reference.clone(),
            })
        );
        assert!(store.load("other-window")?.is_none());

        assert_eq!(
            store.save("kwin-window-private", "rotated-token")?,
            reference
        );
        assert_eq!(
            store
                .load("kwin-window-private")?
                .map(|stored| stored.token),
            Some("rotated-token".to_string())
        );
        let content = fs::read_to_string(&store.path)?;
        assert!(!content.contains("kwin-window-private"));
        assert_eq!(
            fs::metadata(&store.path)?.permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_restore_token_files_with_broad_permissions() -> Result<()> {
        let (root, store) = temporary_store("permissions");
        store.save("window-1", "private-token")?;
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644))?;
        let error = store
            .load("window-1")
            .expect_err("broad token-file mode must fail closed");
        assert!(error.to_string().contains("must not be accessible"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_malformed_or_unknown_restore_state() -> Result<()> {
        let (root, store) = temporary_store("malformed");
        store.save("window-1", "private-token")?;
        fs::write(&store.path, b"{not-json}\n")?;
        let malformed = store
            .load("window-1")
            .expect_err("malformed token state must fail closed");
        assert!(
            malformed
                .to_string()
                .contains("parse capture restore-token file")
        );

        fs::write(
            &store.path,
            b"{\"version\":99,\"target_key\":\"key\",\"token\":\"token\"}\n",
        )?;
        let unknown = store
            .load("window-1")
            .expect_err("unknown token-state version must fail closed");
        assert!(unknown.to_string().contains("unsupported"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_symlinked_files_and_writable_parent_directories() -> Result<()> {
        let (root, store) = temporary_store("symlink");
        let parent = store.path.parent().expect("store has parent");
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let outside = root.join("outside-token");
        fs::write(&outside, b"not private state")?;
        symlink(&outside, &store.path)?;
        let error = store
            .load("window-1")
            .expect_err("symlinked token state must fail closed");
        assert!(error.to_string().contains("regular file"));
        fs::remove_file(&store.path)?;

        fs::set_permissions(parent, fs::Permissions::from_mode(0o777))?;
        let error = store
            .save("window-1", "private-token")
            .expect_err("writable parent must fail closed");
        assert!(error.to_string().contains("writable by group or other"));
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
