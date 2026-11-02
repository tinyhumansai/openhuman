use std::path::{Path, PathBuf};

use super::types::SecurityPolicy;
use tinytools::status::POLICY_BLOCKED_MARKER;

impl SecurityPolicy {
    /// Expand a leading `~/` using the same config helper used by path scope setup.
    pub(super) fn expand_tilde(&self, path: &str) -> String {
        crate::config::expand_tilde(path)
    }

    /// Synchronous lexical preflight only. TinySecurity performs authorization,
    /// protected-floor and reservation decisions in the async validators.
    pub fn is_path_string_allowed(&self, path: &str) -> bool {
        !path.is_empty()
            && !path.contains('\0')
            && !Path::new(path)
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            && !path.to_ascii_lowercase().contains("..%2f")
            && !path.to_ascii_lowercase().contains("%2f..")
    }

    /// Ask TinySecurity whether a path intersects its unconditional protected
    /// floor. An unavailable module refuses access.
    pub async fn is_always_forbidden_async(path: &Path) -> bool {
        #[cfg(feature = "security-module")]
        {
            match crate::modules::security::gap::protected_paths(&[path.to_path_buf()]).await {
                Ok(answers) => answers.first().copied().unwrap_or(true),
                Err(error) => {
                    log::debug!(
                        "[security][path] protected-floor query unavailable; refused: {error}"
                    );
                    true
                }
            }
        }
        #[cfg(not(feature = "security-module"))]
        {
            let _ = path;
            true
        }
    }

    /// Validate a path for file I/O through TinySecurity. Returns the resolved
    /// path the module authorized.
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!("{POLICY_BLOCKED_MARKER} Invalid path"));
        }
        let expanded = self.expand_tilde(path);
        let candidate = if Path::new(&expanded).is_absolute() {
            PathBuf::from(&expanded)
        } else {
            self.action_dir.join(&expanded)
        };
        if let Ok(resolved) = tokio::fs::canonicalize(&candidate).await {
            if self.authorize_ready_media_artifact_path(&resolved).await {
                return self.validate_native_path(path, false).await;
            }
        }
        self.validate_native_path(path, false).await
    }

    /// Resolve a prospective destination parent through TinySecurity's
    /// `ValidateParent` operation.
    pub async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!("{POLICY_BLOCKED_MARKER} Invalid path"));
        }
        self.validate_native_path(path, true).await
    }

    /// Resolve a ready media artifact outside the ordinary tool roots, then
    /// reserve only that exact path as a TinySecurity exception.
    async fn authorize_ready_media_artifact_path(&self, path: &Path) -> bool {
        if Self::is_always_forbidden_async(path).await || self.is_internal(path).await {
            return false;
        }
        let Some(roots) = self
            .media_files_roots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        else {
            self.forget_media_artifact_path(path);
            return false;
        };
        if !roots.contains_path(path) {
            self.forget_media_artifact_path(path);
            return false;
        }
        let artifacts = match crate::artifacts::store::list_artifacts(
            &self.workspace_dir,
            0,
            usize::MAX,
            None,
        )
        .await
        {
            Ok((artifacts, _)) => artifacts,
            Err(error) => {
                tracing::warn!(target: "security", %error, "[security] could not resolve a media artifact read");
                return false;
            }
        };
        for meta in artifacts.iter().filter(|meta| {
            matches!(
                &meta.kind,
                crate::artifacts::ArtifactKind::Image | crate::artifacts::ArtifactKind::Video
            ) && matches!(&meta.status, crate::artifacts::ArtifactStatus::Ready)
        }) {
            let Ok(ready_path) =
                crate::artifacts::resolve_ready_file_meta(&self.workspace_dir, &roots, meta).await
            else {
                continue;
            };
            let Ok(ready_path) = tokio::fs::canonicalize(ready_path).await else {
                continue;
            };
            if ready_path == path {
                self.media_artifact_paths
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(ready_path);
                return true;
            }
        }
        self.forget_media_artifact_path(path);
        false
    }

    fn forget_media_artifact_path(&self, path: &Path) {
        self.media_artifact_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(path);
    }
}
