//! OpenHuman credential adapter for tinyagents cloud embeddings.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::harness::embeddings::{
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
                .map_err(|error| tinyagents::TinyAgentsError::Embedding(error.to_string()))?
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    tinyagents::TinyAgentsError::Validation(
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
mod tests {
    use super::*;

    /// Privacy epic S7 (#4441): under LocalOnly, `embed` refuses before touching
    /// the inner cloud transport (so no network / no auth is needed to observe
    /// the block). Uses the thread-scoped privacy override so it never mutates
    /// the process-global policy that sibling tests read.
    #[tokio::test]
    async fn embed_blocked_under_local_only() {
        let _mode = crate::openhuman::security::live_policy::test_privacy_scope(
            crate::openhuman::config::PrivacyMode::LocalOnly,
        );
        let provider = OpenHumanCloudEmbedding::new(
            Some("http://127.0.0.1:0".into()),
            Some(std::env::temp_dir().join("openhuman_embeddings_localonly_state")),
            false,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        );

        let err = provider.embed(&["hello"]).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("Local-only privacy mode is active"),
            "unexpected error: {err}"
        );
    }

    /// The keyless credential scope must land in the **user-scoped** directory,
    /// never the root. Sign-in writes `auth-profiles.json` to
    /// `{root}/users/<user_id>/`; the root itself holds no such file, so the
    /// previous root-returning implementation made every keyless managed
    /// embedder fail with "No backend session for cloud embeddings" for a user
    /// who was signed in.
    #[test]
    fn default_scope_is_the_active_user_dir_not_the_root() {
        let root = std::path::Path::new("/tmp/openhuman-root");

        let resolved = user_scoped_state_dir(root, Some("user-abc123"));

        assert_eq!(
            resolved,
            root.join("users").join("user-abc123"),
            "managed embedder must read credentials from the active user's dir"
        );
        assert_ne!(
            resolved, root,
            "the root dir holds no auth-profiles.json — resolving to it is the bug"
        );
    }

    /// With no user signed in yet, the scope is the pre-login user dir — the
    /// same directory the pre-login config and its credential store live in.
    /// Falling back to the root here would reintroduce the same empty-store
    /// failure one login earlier.
    #[test]
    fn default_scope_falls_back_to_the_pre_login_user_dir() {
        let root = std::path::Path::new("/tmp/openhuman-root");

        let resolved = user_scoped_state_dir(root, None);

        assert_eq!(
            resolved,
            root.join("users")
                .join(crate::openhuman::config::PRE_LOGIN_USER_ID),
            "a pre-login process must read its own store, not the empty root"
        );
    }

    /// `OPENHUMAN_WORKSPACE` must resolve through the same workspace→config-dir
    /// mapping `config::load` uses, not return the raw workspace path. A legacy
    /// `<X>/workspace` override keeps its credentials in the sibling
    /// `<X>/.openhuman` dir; returning the workspace dir itself would send the
    /// keyless embedder to a directory with no `auth-profiles.json` and
    /// reintroduce "No backend session" for that deployment.
    #[test]
    fn env_workspace_scope_is_the_config_dir_not_the_raw_workspace() {
        // A path that does not exist on disk, so the resolver's `config.toml`
        // probes both miss and the `"workspace"` basename rule decides.
        let workspace = std::path::Path::new("/nonexistent-openhuman-test-root/workspace");

        let resolved = env_workspace_state_dir(workspace);

        assert_eq!(
            resolved,
            std::path::Path::new("/nonexistent-openhuman-test-root/.openhuman"),
            "a `.../workspace` override must resolve to its sibling .openhuman config dir"
        );
        assert_ne!(
            resolved, workspace,
            "returning the raw workspace dir is the regression this guards against"
        );
    }
}
