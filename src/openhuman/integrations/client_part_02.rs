
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
    config: &crate::openhuman::config::Config,
) -> IntegrationPricing {
    use crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT;

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
/// `None` if the user isn't signed in yet.
///
/// Both the backend URL and the auth token come from **core defaults**:
///
/// - backend URL → [`crate::api::config::effective_backend_api_url`]
///   applied to `config.api_url`. Unlike the plain
///   [`crate::api::config::effective_api_url`] resolver (which honours a
///   user-set local-AI endpoint so chat completions still work), the
///   backend resolver detects local-AI URLs and falls back to the
///   `BACKEND_URL` / `VITE_BACKEND_URL` env vars (and finally the hosted
///   default) so backend paths don't get concatenated onto a local
///   Ollama/vLLM endpoint and 404.
/// - auth token → [`crate::api::jwt::get_session_token`], i.e. the
///   app-session JWT written by `auth_store_session` — the same token
///   that billing, team, webhooks, referral, memory, etc. all use.
///
/// There are no per-feature toggles for the shared client itself —
/// callers that need a kill switch (e.g. twilio, google_places,
/// parallel) gate tool registration at their own level.
pub fn build_client(config: &crate::openhuman::config::Config) -> Option<Arc<IntegrationClient>> {
    // Use the integrations-specific resolver: when `config.api_url` is set
    // to a local-AI endpoint (Ollama, vLLM, …), it would still be perfect
    // for `/v1/chat/completions`, but reusing it as the base for backend
    // integration paths produces URLs like
    //   http://127.0.0.1:11434/v1/agent-integrations/composio/toolkits
    // which 404 against the local LLM and flooded Sentry
    // (OPENHUMAN-TAURI-51 / -80 / -7Z). The helper falls through to env /
    // default backend in that case so integrations actually work.
    let backend_url = crate::api::config::effective_backend_api_url(&config.api_url);

    // Primary: app-session JWT from the auth profile store.
    let session_token = match crate::api::jwt::get_session_token(config) {
        Ok(Some(tok)) => {
            let trimmed = tok.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("[integrations] failed to read session token: {e}");
            None
        }
    };

    match session_token {
        Some(token) => {
            tracing::debug!(
                backend_url = %backend_url,
                "[integrations] client built (session token resolved)"
            );
            Some(Arc::new(IntegrationClient::new_with_budget_config(
                backend_url,
                token,
                Arc::new(config.clone()),
            )))
        }
        None => {
            tracing::warn!(
                "[integrations] no auth token available — user is not signed in \
                 (no app-session JWT)"
            );
            None
        }
    }
}
