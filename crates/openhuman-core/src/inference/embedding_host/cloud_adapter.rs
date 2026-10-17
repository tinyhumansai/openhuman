//! OpenHuman credential adapter for tinyagents cloud embeddings.

use std::path::PathBuf;
use std::sync::Arc;

use tinyinference_embeddings::{
    BearerResolver, CloudEmbeddingModel, EmbeddingModel, DEFAULT_CLOUD_DIMENSIONS,
    DEFAULT_CLOUD_MODEL,
};

use crate::security::approval::ApprovalChatContext;
use crate::security::credentials::{AuthService, APP_SESSION_PROVIDER};

pub const DEFAULT_CLOUD_EMBEDDING_MODEL: &str = DEFAULT_CLOUD_MODEL;
pub const DEFAULT_CLOUD_EMBEDDING_DIMENSIONS: usize = DEFAULT_CLOUD_DIMENSIONS;

/// Host-owned credential resolution and egress disclosure around the
/// crate-owned cloud transport.
///
/// The egress guard (policy check, then the `ExternalTransferPending`
/// disclosure) runs here, before the batch reaches the transport, so it can
/// carry the chat of the turn that made the call: an embedder built for a chat
/// turn is [`routed_to`](Self::routed_to) that chat and its disclosure surfaces
/// there. One built without a chat (background sync, the embedding RPC) still
/// discloses, unrouted. Nothing reads the chat from the running task.
pub struct OpenHumanCloudEmbeddingModel {
    inner: Arc<CloudEmbeddingModel>,
    model: String,
    chat: Option<ApprovalChatContext>,
}

impl OpenHumanCloudEmbeddingModel {
    pub fn new(
        api_url: Option<String>,
        openhuman_dir: Option<PathBuf>,
        secrets_encrypt: bool,
        model: impl Into<String>,
        dimensions: usize,
    ) -> Self {
        let state_dir = openhuman_dir.unwrap_or_else(default_state_dir);
        let base = crate::backend::inference_base_url(&api_url).ok();
        let backend_available = base.is_some();
        let base_url = format!(
            "{}/openai/v1",
            base.unwrap_or_default().trim_end_matches('/')
        );
        let key_endpoint = base_url.clone();
        let bearer: BearerResolver = Arc::new(move || {
            if !backend_available {
                return Err(tinyinference_embeddings::Error::Validation(format!(
                    "{} managed embeddings need a backend transport",
                    crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
                )));
            }
            // A stored TinyHumans API key is the bearer outright, exactly as
            // for managed inference (`OpenHumanBackendModel::resolve_bearer`):
            // `/openai/v1/embeddings` accepts it as `Bearer <key>`. Same
            // plaintext guard too — never put the key on a non-HTTPS,
            // non-loopback wire.
            if let Some(key) =
                crate::security::credentials::api_key::get_api_key_in(&state_dir, secrets_encrypt)
                    .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?
            {
                if !crate::inference::provider::openhuman_backend_model::is_managed_endpoint_for_api_key(
                    &key_endpoint,
                ) {
                    return Err(tinyinference_embeddings::Error::Validation(format!(
                        "refusing to send the TinyHumans API key over a non-HTTPS, non-loopback \
                         endpoint: {key_endpoint}"
                    )));
                }
                log::debug!("[embeddings::cloud] authenticating with api-key");
                return Ok(key);
            }
            let auth = AuthService::new(&state_dir, secrets_encrypt);
            auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None)
                .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    tinyinference_embeddings::Error::Validation(
                        "No backend session for cloud embeddings: log in to OpenHuman or set a \
                         TinyHumans API key"
                            .into(),
                    )
                })
        });
        let model = model.into();
        Self {
            inner: Arc::new(CloudEmbeddingModel::new(
                base_url,
                model.clone(),
                dimensions,
                bearer,
            )),
            model,
            chat: None,
        }
    }

    /// This embedder with its egress disclosures routed to `chat` (the turn's
    /// thread and client); `None` leaves them unrouted. Shares the transport,
    /// so it is cheap to do per turn.
    #[must_use]
    pub fn routed_to(&self, chat: Option<ApprovalChatContext>) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            model: self.model.clone(),
            chat,
        }
    }

    /// The egress guard: the privacy policy may refuse the transfer, and an
    /// allowed one is disclosed to the chat this embedder is routed to.
    fn guard(&self) -> tinyinference_embeddings::Result<()> {
        let egress = crate::security::egress::EgressDescriptor::embedding("cloud", &self.model);
        crate::security::egress::enforce_egress(&egress)
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?;
        crate::security::egress::emit_external_transfer(self.chat.as_ref(), egress);
        Ok(())
    }

    pub fn from_config(config: &crate::config::Config, model: &str, dimensions: usize) -> Self {
        Self::new(
            config.api_url.clone(),
            Some(crate::security::credentials::state_dir_from_config(config)),
            config.secrets.encrypt,
            model,
            dimensions,
        )
    }

    pub fn new_default_scope(model: &str, dimensions: usize) -> Self {
        Self::new(None, None, true, model, dimensions)
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

#[async_trait::async_trait]
impl EmbeddingModel for OpenHumanCloudEmbeddingModel {
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

    async fn embed(&self, texts: &[String]) -> tinyinference_embeddings::Result<Vec<Vec<f32>>> {
        // Only a batch that will actually leave is guarded: an empty one
        // answers locally, and a blank input is refused by the transport.
        if !texts.is_empty() && texts.iter().all(|text| !text.trim().is_empty()) {
            self.guard()?;
        }
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
#[path = "cloud_adapter_tests.rs"]
mod tests;
