//! Direct-mode reads: `direct_list_connections` and `direct_list_tools`.
//!
//! The v3 requests and their reshaping into the canonical envelopes run in the
//! connector module, reached over the bus with `ListConnectionsDirect` and
//! `ListToolsDirect`. Those members carry the credential on the request, so the
//! module's configured route is untouched and nothing is persisted (which is
//! also how `composio.set_api_key` checks a key before storing it).
//!
//! What stays here is host policy: the process-wide invalid-key gate
//! (`direct_auth`) and its user-facing messages, the host's proxy and TLS policy
//! (handed to the module with the credential, see `network`), and the identity
//! fields the host fills in itself.

use std::sync::Arc;

use tinyconnectors_bus::{ComposioDirectConnectionsRequest, ComposioDirectToolsRequest};

use super::super::direct_auth;
use super::super::module_client::{self};
use super::super::types::{ComposioConnectionsResponse, ComposioToolsResponse};
use super::DirectCredential;
use tinyconnectors_bus::names::methods;

/// `ListConnectionsDirect`, with the member failure peeled down to the message
/// the user is shown.
async fn module_list_connections(
    config: &crate::config::Config,
    direct: &DirectCredential,
) -> anyhow::Result<ComposioConnectionsResponse> {
    let request = ComposioDirectConnectionsRequest {
        credential: direct.module_credential(),
    };
    let mut response: ComposioConnectionsResponse =
        module_client::call_stateless(config, methods::LIST_CONNECTIONS_DIRECT, request)
            .await
            .map_err(|error| {
                anyhow::anyhow!(module_client::member_failure_message(
                    methods::LIST_CONNECTIONS_DIRECT,
                    &error
                ))
            })?;
    // Identity fields are the host's to fill (`enrich_connections_with_identity`
    // from cached profile data); the module also lifts them from the v3 row.
    for connection in &mut response.connections {
        connection.account_email = None;
        connection.workspace = None;
        connection.username = None;
    }
    Ok(response)
}

/// Direct-mode connection listing (Composio v3 `/connected_accounts`), gated
/// by the host's invalid-key breaker. Rows come back as canonical
/// [`ComposioConnection`](super::super::types::ComposioConnection)s; a malformed
/// row is kept with empty fields and reads as inactive (fail-safe).
pub async fn direct_list_connections(
    config: &crate::config::Config,
    direct: &Arc<DirectCredential>,
) -> anyhow::Result<ComposioConnectionsResponse> {
    tracing::debug!("[composio-direct] list_connections: GET v3 /connected_accounts");
    let key_id = direct.auth_key_fingerprint();
    if let Some(error) = direct_auth::direct_auth_backoff_error(key_id) {
        tracing::warn!(
            "[composio-direct] list_connections: direct API key backoff gate open; \
             skipping v3 /connected_accounts"
        );
        anyhow::bail!("{error}");
    }

    let response = match module_list_connections(config, direct).await {
        Ok(response) => {
            direct_auth::record_direct_auth_success(key_id);
            response
        }
        Err(error) => {
            let rendered = format!("{error:#}");
            match direct_auth::record_direct_auth_failure(key_id, &rendered) {
                direct_auth::DirectAuthFailureDecision::NotAuthFailure => {}
                direct_auth::DirectAuthFailureDecision::RetryAllowed { consecutive } => {
                    tracing::warn!(
                        consecutive,
                        threshold = direct_auth::DIRECT_INVALID_API_KEY_THRESHOLD,
                        "[composio-direct] list_connections: direct API key rejected"
                    );
                }
                direct_auth::DirectAuthFailureDecision::CircuitOpened { consecutive } => {
                    let backoff = direct_auth::invalid_api_key_backoff_message(consecutive);
                    tracing::warn!(
                        consecutive,
                        threshold = direct_auth::DIRECT_INVALID_API_KEY_THRESHOLD,
                        "[composio-direct] list_connections: direct API key backoff gate opened"
                    );
                    anyhow::bail!("{backoff}");
                }
            }
            return Err(error);
        }
    };
    tracing::debug!(
        count = response.connections.len(),
        "[composio-direct] list_connections: mapped v3 connected accounts"
    );
    Ok(response)
}

/// Direct-mode tool listing. Calls Composio v3 `/tools` through the module (it sends
/// `limit=200`, `toolkit_versions=latest`, `toolkits=<csv>` and repeated
/// `tags=`) and returns the same `ComposioToolSchema` envelope the
/// backend-proxied path returns.
///
/// `toolkits` may be empty (full direct-tenant catalogue) or scoped to
/// the user's connected toolkits (preferred — keeps response size bounded
/// and skips schemas the agent can't actually call). `composio_list_tools`'s
/// direct branch passes `direct_list_connections`'s active set.
///
/// `tags` mirrors the backend path's tag filter so a self-key user's
/// `composio_list_tools(..., tags)` request narrows by Composio action tag
/// in direct mode too (previously the tag filter was silently dropped on
/// the direct branch). The caller is expected to have already applied
/// [`crate::integrations::composio::ops::should_forward_tags`] before passing `tags` here.
///
/// Schemas surfaced here are tenant-agnostic — Composio's action
/// definitions are the same across tenants, so direct-mode users get
/// the same model-callable shape backend-mode does. Downstream curated-
/// whitelist filtering (`evaluate_tool_visibility` / `find_curated`)
/// still applies at the `ops::composio_list_tools` layer.
///
/// `pub(crate)` (widened from `pub(super)`) so
/// `catalog::fetch_raw_toolkit_tools` can call this directly for
/// the LIVE (uncurated) tool-contract catalog the Workflow builder grounds
/// against — that caller deliberately bypasses `composio_list_tools`'s
/// curated-whitelist filter (`filter_list_tools_response_for_direct`),
/// which this function never applies itself; the filter is layered on by
/// its `composio_list_tools` caller, not baked in here.
pub(crate) async fn direct_list_tools(
    config: &crate::config::Config,
    direct: &Arc<DirectCredential>,
    toolkits: &[String],
    tags: Option<&[String]>,
) -> anyhow::Result<ComposioToolsResponse> {
    tracing::debug!(
        toolkits = toolkits.len(),
        tags = tags.map(<[String]>::len).unwrap_or(0),
        "[composio-direct] list_tools: GET v3 /tools"
    );
    let request = ComposioDirectToolsRequest {
        credential: direct.module_credential(),
        toolkits: toolkits.to_vec(),
        tags: tags.unwrap_or(&[]).to_vec(),
    };
    let response: ComposioToolsResponse =
        module_client::call_stateless(config, methods::LIST_TOOLS_DIRECT, request)
            .await
            .map_err(|error| {
                anyhow::anyhow!(module_client::member_failure_message(
                    methods::LIST_TOOLS_DIRECT,
                    &error
                ))
            })?;
    tracing::debug!(
        count = response.tools.len(),
        "[composio-direct] list_tools: mapped v3 tool schemas"
    );
    Ok(response)
}
