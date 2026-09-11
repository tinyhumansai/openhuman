//! OpenHuman credential adapter for tinyagents cloud embeddings.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinyinference::embeddings::{
    BearerResolver, CloudEmbeddingModel, DEFAULT_CLOUD_DIMENSIONS, DEFAULT_CLOUD_MODEL,
};

use super::{EmbeddingProvider, TinyAgentsEmbeddingProvider};
use crate::api::config::effective_api_url;
use crate::openhuman::security::credentials::{AuthService, APP_SESSION_PROVIDER};

pub const DEFAULT_CLOUD_EMBEDDING_MODEL: &str = DEFAULT_CLOUD_MODEL;
pub const DEFAULT_CLOUD_EMBEDDING_DIMENSIONS: usize = DEFAULT_CLOUD_DIMENSIONS;

/// Host-owned credential resolution around the crate-owned cloud transport.
pub struct OpenHumanCloudEmbedding {
    inner: TinyAgentsEmbeddingProvider,
}

impl OpenHumanCloudEmbedding {
    pub fn new(
        api_url: Option<String>,
        openhuman_dir: Option<PathBuf>,
        secrets_encrypt: bool,
        model: impl Into<String>,
        dimensions: usize,
    ) -> Self {
        let state_dir = openhuman_dir.unwrap_or_else(default_state_dir);
        let bearer: BearerResolver = Arc::new(move || {
            let auth = AuthService::new(&state_dir, secrets_encrypt);
            auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None)
                .map_err(|error| tinyinference::Error::Embedding(error.to_string()))?
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    tinyinference::Error::Validation(
                        "No backend session for cloud embeddings: log in to OpenHuman".into(),
                    )
                })
        });
        let base_url = format!(
            "{}/openai/v1",
            effective_api_url(&api_url).trim_end_matches('/')
        );
        Self {
            inner: TinyAgentsEmbeddingProvider::new(CloudEmbeddingModel::new(
                base_url, model, dimensions, bearer,
            )),
        }
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

    let root = crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|error| {
        log::warn!(
            "[embeddings::cloud] could not resolve the openhuman root dir ({error}); \
             falling back to a relative .openhuman path"
        );
        PathBuf::from(".openhuman")
    });

    // Never log the resolved path or the user id: both identify the user.
    let user_id = crate::openhuman::config::read_active_user_id(&root);
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
    let (config_dir, _workspace_dir) =
        crate::openhuman::config::resolve_config_dir_for_workspace(workspace);
    config_dir
}

/// Pure core of [`default_state_dir`]'s non-env branch, split out so the
/// user-scoping invariant is unit-testable without a home directory or a real
/// `active_user.toml`.
fn user_scoped_state_dir(root: &std::path::Path, active_user_id: Option<&str>) -> PathBuf {
    crate::openhuman::config::user_openhuman_dir(
        root,
        active_user_id.unwrap_or(crate::openhuman::config::PRE_LOGIN_USER_ID),
    )
}

#[async_trait]
impl EmbeddingProvider for OpenHumanCloudEmbedding {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn signature(&self) -> String {
        self.inner.signature()
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        // Egress spine (privacy epic S2, #4436): the input texts leave the
        // device for the cloud embedding backend — disclose before the request.
        let egress = crate::openhuman::security::egress::EgressDescriptor::embedding(
            "cloud",
            self.inner.model_id(),
        );
        // Local-only enforcement (privacy epic S7, #4441): refuse cloud
        // embedding under LocalOnly before disclosing or sending the texts.
        crate::openhuman::security::egress::enforce_egress(&egress)?;
        crate::openhuman::security::egress::emit_external_transfer(egress);
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
#[path = "cloud_adapter_tests.rs"]
mod tests;
