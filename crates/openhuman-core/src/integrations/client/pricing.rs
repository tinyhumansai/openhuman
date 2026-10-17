//! Pricing cache and the top-level `build_client` / `pricing_for_config`
//! helpers used to construct an [`IntegrationClient`] from app config.

use std::sync::Arc;

use crate::integrations::types::IntegrationPricing;

use super::construct::IntegrationClient;

impl IntegrationClient {
    /// Fetch and cache pricing info from the backend. Returns a default
    /// (empty) pricing struct on network errors so tool registration never fails.
    pub async fn pricing(&self) -> &IntegrationPricing {
        self.pricing
            .get_or_init(|| async {
                match self
                    .get::<IntegrationPricing>(
                        &crate::security::approval::ApprovalTurnContext::default(),
                        "/agent-integrations/pricing",
                    )
                    .await
                {
                    Ok(p) => {
                        tracing::debug!("[integrations] pricing fetched successfully");
                        p
                    }
                    Err(e) => {
                        tracing::warn!("[integrations] failed to fetch pricing: {e}");
                        IntegrationPricing::default()
                    }
                }
            })
            .await
    }
}

/// Fetch pricing for the integrations module, honouring the
/// Composio routing mode.
///
/// When `config.composio.mode == "direct"`, the user is running with
/// their own Composio API key and there is **no backend session** that
/// could serve `/agent-integrations/pricing` — the backend route is
/// what mediates the margin between Composio's raw price and what the
/// hosted product charges. In direct mode, margins do not apply
/// (the user pays Composio directly) and the backend may not even be
/// reachable (sovereign / offline-friendly deployments). We
/// short-circuit to the default empty pricing struct and emit a
/// `[composio-direct]` log line so this branch is easy to grep.
///
/// In backend mode we fall through to the live cache on
/// [`IntegrationClient::pricing`], preserving the existing behavior
/// for every caller. The empty default struct is identical to what
/// [`IntegrationClient::pricing`] returns on a network error, so
/// downstream consumers don't need a separate code path.
pub async fn pricing_for_config(
    client: &IntegrationClient,
    config: &crate::config::Config,
) -> IntegrationPricing {
    use crate::config::schema::COMPOSIO_MODE_DIRECT;

    if config.composio.mode.trim() == COMPOSIO_MODE_DIRECT {
        tracing::debug!(
            "[composio-direct] pricing short-circuit: backend `/agent-integrations/pricing` \
             is unreachable in direct mode — returning default (empty) pricing"
        );
        return IntegrationPricing::default();
    }
    client.pricing().await.clone()
}

/// Helper: build an `Arc<IntegrationClient>` from the root config, or
/// `None` if there is no backend credential. A local offline credential keeps
/// the core signed in but cannot authenticate hosted integration routes.
///
/// Both the backend URL and the credential come from **core defaults**:
///
/// - backend URL → [`crate::backend::base_url`]
///   applied to `config.api_url`. Unlike the plain
///   [`crate::backend::inference_base_url`] resolver (which honours a
///   user-set local-AI endpoint so chat completions still work), the
///   backend resolver detects local-AI URLs and falls back to the
///   `BACKEND_URL` / `VITE_BACKEND_URL` env vars (and finally the hosted
///   default) so backend paths don't get concatenated onto a local
///   Ollama/vLLM endpoint and 404.
/// - credential → [`resolve_backend_credential`], the same resolver every
///   other backend caller uses: the stored TinyHumans API key when there is
///   one (sent as `x-api-key`), else the live app-session JWT (sent as
///   `Authorization: Bearer`). The local offline token and an expired session
///   both resolve to an error, so no client is built for them.
///
/// There are no per-feature toggles for the shared client itself —
/// callers that need a kill switch (e.g. google_places, parallel,
/// stock_prices) gate tool registration at their own level.
///
/// [`resolve_backend_credential`]: crate::security::credentials::session_support::resolve_backend_credential
pub fn build_client(config: &crate::config::Config) -> Option<Arc<IntegrationClient>> {
    // Use the integrations-specific resolver: when `config.api_url` is set
    // to a local-AI endpoint (Ollama, vLLM, …), it would still be perfect
    // for `/v1/chat/completions`, but reusing it as the base for backend
    // integration paths produces URLs like
    //   http://127.0.0.1:11434/v1/agent-integrations/composio/toolkits
    // which 404 against the local LLM and flooded Sentry
    // (OPENHUMAN-TAURI-51 / -80 / -7Z). The helper falls through to env /
    // default backend in that case so integrations actually work.
    let backend_url = match crate::backend::base_url(&config.api_url) {
        Ok(url) => url,
        Err(_) => {
            tracing::debug!(
                "[integrations] no backend transport — integrations client unavailable"
            );
            return None;
        }
    };

    let credential =
        match crate::security::credentials::session_support::resolve_backend_credential(config) {
            Ok(credential) => Some(credential),
            Err(e) => {
                tracing::debug!("[integrations] no backend credential: {e}");
                None
            }
        };

    build_client_with_credential(config, backend_url, credential)
}

/// The credential decision is separate from profile-store lookup so callers
/// can test the backend boundary without process-wide auth-store state.
fn build_client_with_credential(
    config: &crate::config::Config,
    backend_url: String,
    credential: Option<crate::security::credentials::session_support::BackendCredential>,
) -> Option<Arc<IntegrationClient>> {
    use crate::security::credentials::session_support::BackendCredential;

    let credential = match credential {
        Some(BackendCredential::Session(token))
            if crate::security::credentials::session_support::is_local_session_token(
                token.trim(),
            ) =>
        {
            // Offline identity is valid for the local core, but the hosted
            // integrations API cannot authenticate it. Never send it as a
            // backend JWT: a predictable 401 would publish a global
            // SessionExpired event into an otherwise healthy local chat.
            tracing::debug!(
                "[integrations] local offline credential has no hosted integrations client"
            );
            return None;
        }
        Some(BackendCredential::Session(token)) => {
            BackendCredential::Session(token.trim().to_owned())
        }
        Some(BackendCredential::ApiKey(key)) => BackendCredential::ApiKey(key.trim().to_owned()),
        None => return None,
    };
    if credential.secret().is_empty() {
        tracing::warn!("[integrations] no auth token available — not signed in and no API key set");
        return None;
    }
    tracing::debug!(
        backend_url = %backend_url,
        api_key = credential.is_api_key(),
        "[integrations] client built (backend credential resolved)"
    );
    Some(Arc::new(
        IntegrationClient::new_with_credential_and_budget_config(
            backend_url,
            credential,
            Arc::new(config.clone()),
        ),
    ))
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
