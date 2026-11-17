//! [`FileConfigSource`]: the config as `config.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use super::{ConfigRead, ConfigSource};
use crate::config::schema::load::atomic_commit::commit_replacement;
use crate::config::schema::load::impl_load::read_config_with_recovery_or_default;

/// A config at a `config.toml` path.
///
/// Reads keep the file's corruption recovery (non-UTF-8 content is renamed
/// aside and the `.bak` tried). Writes stage a `0600` temp file, fsync it, and
/// hand it to [`commit_replacement`], which keeps the previous config as
/// `.bak`. The file stays hand-editable TOML.
#[derive(Debug, Clone)]
pub(crate) struct FileConfigSource {
    path: PathBuf,
}

impl FileConfigSource {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl ConfigSource for FileConfigSource {
    fn label(&self) -> &'static str {
        "file"
    }

    fn anchor(&self) -> super::ConfigSourceAnchor {
        super::ConfigSourceAnchor::File(super::normalized_path(&self.path))
    }

    async fn exists(&self) -> Result<bool> {
        fs::try_exists(&self.path)
            .await
            .with_context(|| format!("Failed to check the config file: {}", self.path.display()))
    }

    async fn read(&self) -> Result<ConfigRead> {
        let (contents, recovered) = read_config_with_recovery_or_default(&self.path).await?;
        Ok(ConfigRead {
            contents,
            recovered,
            from_document: false,
            document_version: None,
            source_anchor: self.anchor(),
        })
    }

    async fn write(&self, toml_str: &str) -> Result<()> {
        let config_path = &self.path;
        let parent_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        fs::create_dir_all(parent_dir).await.with_context(|| {
            format!(
                "Failed to create config directory: {}",
                parent_dir.display()
            )
        })?;

        // Built from the original `OsStr`, so a non-UTF-8 file name keeps its
        // own temp and backup names instead of collapsing onto `config.toml`.
        let file_name = config_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("config.toml"));
        let with_name = |prefix: &str, suffix: String| {
            let mut name = std::ffi::OsString::from(prefix);
            name.push(file_name);
            name.push(suffix);
            parent_dir.join(name)
        };
        let temp_path = with_name(".", format!(".tmp-{}", uuid::Uuid::new_v4()));
        let backup_path = with_name("", ".bak".to_string());

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to create temporary config file: {}",
                    temp_path.display()
                )
            })?;

        // Harden BEFORE any secret bytes are written. `create_new` opens at
        // `0o666 & ~umask` (0644 under the usual 022), and the atomic rename
        // below carries the *temp file's* mode onto the live config — so
        // without this every save silently re-widened a config that holds
        // `enc2:` provider keys and channel tokens back to world-readable, and
        // `fs::copy` propagated the same mode onto `config.toml.bak`. The
        // load-time auto-fix only ever repaired it on the next startup.
        //
        // Non-fatal by design: filesystems that do not implement chmod
        // (CIFS/SMB, exFAT, some FUSE mounts) would otherwise turn a save that
        // has always worked into a hard failure. A failed hardening leaves the
        // file exactly as permissive as it was before this call existed, so
        // warn and continue rather than regress writability.
        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            if let Err(e) = fs::set_permissions(&temp_path, Permissions::from_mode(0o600)).await {
                tracing::warn!(
                    path = %temp_path.display(),
                    error = %e,
                    "[security][config] could not restrict config file to 0600; \
                     it may be readable by other local users"
                );
            }
        }

        // The staged file holds the whole config, secrets included: remove it
        // on every failure before the commit instead of leaving it behind.
        let staged = async {
            temp_file
                .write_all(toml_str.as_bytes())
                .await
                .context("Failed to write temporary config contents")?;
            temp_file
                .sync_all()
                .await
                .context("Failed to fsync temporary config file")
        }
        .await;
        drop(temp_file);
        if let Err(error) = staged {
            let _ = fs::remove_file(&temp_path).await;
            return Err(error);
        }

        // Everything above can still fail with the live config untouched.
        // `commit_replacement` owns the swap, and returns `Err` only while the
        // old config is still in place — see its docs for why callers that roll
        // back on `Err` depend on that.
        let committed = commit_replacement(&temp_path, config_path, parent_dir, &backup_path).await;
        if committed.is_err() {
            // The helper removes the staged file after a failed rename but can
            // return earlier (directory recreation); either way it must not be
            // left behind holding the whole config.
            let _ = fs::remove_file(&temp_path).await;
        }
        committed
    }
}
