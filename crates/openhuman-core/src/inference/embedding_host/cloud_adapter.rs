//! Credential scope and the unavailable placeholder for the managed cloud
//! embedder. The embedder itself is a registered provider
//! ([`crate::inference::registry::register_embedding`]); with none registered
//! the managed embedder is a placeholder that answers `BACKEND_UNAVAILABLE:`.

use std::path::PathBuf;

use tinyinference_embeddings::{EmbeddingModel, DEFAULT_CLOUD_DIMENSIONS, DEFAULT_CLOUD_MODEL};

pub const DEFAULT_CLOUD_EMBEDDING_MODEL: &str = DEFAULT_CLOUD_MODEL;
pub const DEFAULT_CLOUD_EMBEDDING_DIMENSIONS: usize = DEFAULT_CLOUD_DIMENSIONS;

/// The managed embedder when no provider is registered for it: keeps the
/// configured identity (name, model, dimensions) so callers that only describe
/// the embedder still work, and refuses to embed with the `BACKEND_UNAVAILABLE:`
/// sentinel.
pub struct UnavailableEmbeddingModel {
    model: String,
    dimensions: usize,
}

impl UnavailableEmbeddingModel {
    pub fn new(model: &str, dimensions: usize) -> Self {
        Self {
            model: model.to_string(),
            dimensions,
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingModel for UnavailableEmbeddingModel {
    fn name(&self) -> &str {
        "cloud"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn signature(&self) -> String {
        format!("unavailable:{}:{}", self.model, self.dimensions)
    }

    async fn embed(&self, _texts: &[String]) -> tinyinference_embeddings::Result<Vec<Vec<f32>>> {
        Err(tinyinference_embeddings::Error::Validation(format!(
            "{} managed embeddings need a hosted provider; none is registered",
            crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
        )))
    }
}

/// The credential directory and encryption flag for a managed embedder:
/// the config's scope when the caller has one, else the process default.
pub(super) fn credential_scope(config: Option<&crate::config::Config>) -> (PathBuf, bool) {
    match config {
        Some(config) => (
            crate::security::credentials::state_dir_from_config(config),
            config.secrets.encrypt,
        ),
        None => (default_state_dir(), true),
    }
}

/// Credential scope used when the caller passes `openhuman_dir = None`.
///
/// `None` means "wherever this process keeps its credentials", and on a shipped
/// desktop that is **not** the root `~/.openhuman`. Sign-in stores the
/// `app-session` token through `AuthService::from_config`, whose state dir is
/// `config.config_path.parent()` — the user-scoped
/// `~/.openhuman/users/<user_id>/`. This function previously returned the root,
/// so every keyless managed embedder resolved a directory with no
/// `auth-profiles.json` in it and a signed-in user's embeds failed with
/// "No backend session for cloud embeddings" on every call.
///
/// Resolution mirrors `config::load`'s own directory choice:
/// 1. `OPENHUMAN_WORKSPACE` when set — resolved through the **same**
///    workspace→config-dir mapping `config::load` uses
///    (`resolve_config_dir_for_workspace`), not the raw env value. A legacy
///    `.../workspace` override maps back to its sibling `.openhuman` root, which
///    is where `auth-profiles.json` actually lives; returning the workspace dir
///    itself would reintroduce the "No backend session" failure for that
///    deployment.
/// 2. otherwise `{root}/users/{active_user_id}`, falling back to the pre-login
///    user (`users/local`) when no user has signed in yet — the same directory
///    the pre-login config was written to, so a pre-login process still reads
///    its own store instead of an empty root.
///
/// Callers holding a `&Config` should still pass the scope explicitly
/// (`create_embedding_provider_with_config`); this is the best available
/// resolution for the call sites that have no `Config` in scope.
fn default_state_dir() -> PathBuf {
    log::debug!("[embeddings::cloud] default credential scope: resolving");
    if let Some(workspace) = std::env::var_os("OPENHUMAN_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        // Never log the resolved path: it identifies the user's home layout.
        log::debug!(
            "[embeddings::cloud] default credential scope = OPENHUMAN_WORKSPACE-derived config dir (env-scoped deployment)"
        );
        return env_workspace_state_dir(&workspace);
    }

    let root = crate::config::default_root_openhuman_dir().unwrap_or_else(|error| {
        log::warn!(
            "[embeddings::cloud] could not resolve the openhuman root dir ({error}); \
             falling back to a relative .openhuman path"
        );
        PathBuf::from(".openhuman")
    });

    // Never log the resolved path or the user id: both identify the user.
    let user_id = crate::config::read_active_user_id(&root);
    log::debug!(
        "[embeddings::cloud] default credential scope resolved = user-scoped dir (active_user_present={})",
        user_id.is_some()
    );
    user_scoped_state_dir(&root, user_id.as_deref())
}

/// Pure core of [`default_state_dir`]'s `OPENHUMAN_WORKSPACE` branch, split out
/// so the workspace→config-dir invariant is unit-testable without touching the
/// process environment.
///
/// Mirrors `config::load`: the credential scope for a workspace override is the
/// config dir [`resolve_config_dir_for_workspace`] derives from it — for a
/// legacy `.../workspace` path that is the sibling `.openhuman` root (which
/// holds `auth-profiles.json`), **not** the workspace dir (which holds none).
fn env_workspace_state_dir(workspace: &std::path::Path) -> PathBuf {
    let (config_dir, _workspace_dir) = crate::config::resolve_config_dir_for_workspace(workspace);
    config_dir
}

/// Pure core of [`default_state_dir`]'s non-env branch, split out so the
/// user-scoping invariant is unit-testable without a home directory or a real
/// `active_user.toml`.
fn user_scoped_state_dir(root: &std::path::Path, active_user_id: Option<&str>) -> PathBuf {
    crate::config::user_openhuman_dir(
        root,
        active_user_id.unwrap_or(crate::config::PRE_LOGIN_USER_ID),
    )
}

#[cfg(test)]
#[path = "cloud_adapter_tests.rs"]
mod tests;
