
/// Backend-mode [`ComposioClient`] constructor. **Internal to the
/// composio module** — external callers should use
/// [`create_composio_client`] (factory) or
/// [`crate::openhuman::agent::harness::subagent_runner::user_is_signed_in_to_composio`]
/// (probe) instead.
///
/// Direct exposure leaked through several call sites during the early
/// direct-mode rollout (#1710), where the backend-only nature caused
/// direct-mode users to false-negative the "signed in" check (the
/// agent-tool registration gate, slack sync RPC, `tools.composio_execute`
/// controller, and heartbeat calendar collector all silently dropped
/// direct-mode users). Locking down here prevents future regressions —
/// any new probe or execution path is forced through the mode-aware
/// surface.
///
/// Composio is **always enabled** — there are no configuration flags
/// gating it. The backend URL and auth token come from the shared
/// core defaults (`config.api_url` plus the app-session JWT) via
/// [`crate::openhuman::integrations::build_client`]. The only reason
/// this returns `None` is that the user isn't signed in to the backend
/// (no JWT). Direct-mode availability is orthogonal — see
/// [`create_composio_client`].
pub(super) fn build_composio_client(
    config: &crate::openhuman::config::Config,
) -> Option<ComposioClient> {
    let inner = crate::openhuman::integrations::build_client(config)?;
    Some(ComposioClient::new(inner))
}

// ── Direct-mode factory ─────────────────────────────────────────────
//
// Mirrors `src/openhuman/inference/embeddings/factory.rs` so anyone reading both
// can pattern-match between domains: string-matched mode, explicit error
// on unknown mode, explicit error when `direct` is selected without an
// API key.

use crate::openhuman::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};

// Re-declare the mode strings as local consts so they can be used as
// pattern arms in the `match` below. `use` imports of `pub const &str`
// values get treated as fresh variable bindings in pattern position
// (Rust's pattern grammar accepts only path-qualified constants), so
// pulling them in here resolves to the same `&'static str` values
// without the "unreachable pattern" warning chain.
const MODE_BACKEND_PAT: &str = COMPOSIO_MODE_BACKEND;
const MODE_DIRECT_PAT: &str = COMPOSIO_MODE_DIRECT;

/// Tagged variant returned by [`create_composio_client`].
///
/// `Backend` wraps the existing backend-proxied [`ComposioClient`]
/// (calls `api.tinyhumans.ai/agent-integrations/composio/*`).
///
/// `Direct` wraps the existing direct-mode HTTP wrapper from
/// `composio/tools/direct.rs` that calls
/// `https://backend.composio.dev/api/v{2,3}` with `x-api-key`. The
/// direct client does not currently cover every endpoint the
/// backend-proxied path exposes (no per-toolkit allowlist, no
/// HMAC-verified trigger fan-out, no `/agent-integrations/pricing`),
/// so most existing call-sites continue to use `Backend` for now.
/// Direct-mode integration of the full surface (especially trigger
/// webhooks) is a follow-up.
pub enum ComposioClientKind {
    Backend(ComposioClient),
    /// Held inside an `Arc` so the variant stays cheap to clone — this
    /// matches the rest of the tool registry which juggles
    /// `Arc<dyn Tool>` for the same direct-mode tool elsewhere.
    Direct(Arc<crate::openhuman::tools::ComposioTool>),
}

pub(crate) fn create_direct_composio_tool_for_api_key(
    config: &crate::openhuman::config::Config,
    api_key: &str,
) -> anyhow::Result<Arc<crate::openhuman::tools::ComposioTool>> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        anyhow::bail!("composio direct api key must not be empty");
    }

    // The direct client takes a `SecurityPolicy` for `Tool::execute`
    // gating, but the factory's job is only to materialize a *client*
    // — it does not actually invoke `execute()` itself, so the
    // default policy is sufficient here. Callers that go through
    // the `Tool` surface re-acquire the live policy from their own
    // context.
    let security = Arc::new(crate::openhuman::security::SecurityPolicy::default());
    #[cfg(debug_assertions)]
    let tool = match (
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2").ok(),
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3").ok(),
    ) {
        (Some(base_v2), Some(base_v3)) => {
            crate::openhuman::tools::ComposioTool::new_with_base_urls_for_loopback(
                api_key,
                Some(config.composio.entity_id.as_str()),
                security,
                base_v2,
                base_v3,
            )
            .map_err(|e| {
                anyhow::anyhow!("invalid debug composio direct loopback base override: {e}")
            })?
        }
        _ => crate::openhuman::tools::ComposioTool::new(
            api_key,
            Some(config.composio.entity_id.as_str()),
            security,
        ),
    };
    #[cfg(not(debug_assertions))]
    let tool = crate::openhuman::tools::ComposioTool::new(
        api_key,
        Some(config.composio.entity_id.as_str()),
        security,
    );
    Ok(Arc::new(tool))
}

impl ComposioClientKind {
    /// Returns `"backend"` or `"direct"` — handy for logging and tests.
    pub fn mode(&self) -> &'static str {
        match self {
            ComposioClientKind::Backend(_) => COMPOSIO_MODE_BACKEND,
            ComposioClientKind::Direct(_) => COMPOSIO_MODE_DIRECT,
        }
    }
}

/// Construct a [`ComposioClientKind`] from the root config.
///
/// Supported `config.composio.mode` values:
///
/// - `"backend"` (default) — backend-proxied; identical to
///   [`build_composio_client`]. Returns
///   `Err("no backend session")` when the user is not signed in.
/// - `"direct"` — BYO key against `backend.composio.dev`. Requires a
///   stored Composio API key under the
///   [`crate::openhuman::security::credentials::COMPOSIO_DIRECT_PROVIDER`]
///   slot **or** an `api_key` value in `config.composio.api_key`. The
///   stored key takes precedence so the encrypted keychain remains the
///   source of truth — `config.toml` is a fallback for power users.
///
/// Any other mode string is rejected with an explicit error so a typo
/// in `config.toml` fails loud instead of silently downgrading.
pub fn create_composio_client(
    config: &crate::openhuman::config::Config,
) -> anyhow::Result<ComposioClientKind> {
    let mode = config.composio.mode.trim();
    tracing::debug!(mode = %mode, "[composio-factory] resolving client");

    match mode {
        // Empty string is treated as the default for forward compatibility
        // with hand-edited configs that omit the field — `serde(default)`
        // already gives us "backend" for missing fields, but a literal
        // empty string in TOML would otherwise be rejected.
        "" | MODE_BACKEND_PAT => {
            let client = build_composio_client(config).ok_or_else(|| {
                anyhow::anyhow!(
                    "composio backend mode unavailable: no backend session token. \
                     Sign in first (auth_store_session)."
                )
            })?;
            tracing::debug!("[composio-factory] resolved backend variant");
            Ok(ComposioClientKind::Backend(client))
        }
        MODE_DIRECT_PAT => {
            // Prefer keychain-stored key; fall back to `config.toml`.
            let stored = crate::openhuman::security::credentials::get_composio_api_key(config)
                .map_err(|e| anyhow::anyhow!("failed to read stored composio api key: {e}"))?;
            let api_key = stored
                .or_else(|| {
                    config
                        .composio
                        .api_key
                        .as_ref()
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "composio direct mode selected but no api key is configured \
                         (set via composio.set_api_key RPC or config.composio.api_key)"
                    )
                })?;

            let tool = create_direct_composio_tool_for_api_key(config, &api_key)?;
            tracing::debug!(
                key_len = api_key.len(),
                "[composio-factory] resolved direct variant (key redacted)"
            );
            Ok(ComposioClientKind::Direct(tool))
        }
        unknown => {
            tracing::warn!(mode = %unknown, "[composio-factory] unknown composio mode");
            Err(anyhow::anyhow!(
                "unknown composio mode: \"{unknown}\". Supported: \"backend\", \"direct\""
            ))
        }
    }
}

// ── Direct-mode response reshapers ──────────────────────────────────
//
// The direct-mode `ComposioTool` (in `composio/tools/direct.rs`)
// speaks `backend.composio.dev/api/v3/*` natively. The helpers below
// reshape those v3 responses into the same envelopes the
// backend-proxied [`ComposioClient`] returns, so callers in `ops.rs` /
// `tools.rs` don't have to branch on mode for downstream concerns
// (event-bus shape, log format, frontend type contract).
//
// All three helpers live next to the factory so anyone touching the
// direct-mode plumbing can see the full envelope-translation surface
// in one place.

use super::{direct_auth, types::ComposioConnection};

/// Direct-mode counterpart to [`ComposioClient::authorize`]. Calls
/// Composio v3 `/connected_accounts/link` via
/// [`crate::openhuman::tools::ComposioTool::get_connection_url`] and
/// reshapes the response into the [`ComposioAuthorizeResponse`] the
/// backend-proxied path emits.
///
/// The v3 endpoint returns a redirect URL but does NOT (currently)
/// surface a stable `connection_id` in the same call — the connection
/// row is created lazily when the user completes OAuth on Composio's
/// hosted page. To preserve the response contract the frontend already
/// consumes, we emit an empty `connection_id` for now. The 5 s
/// `list_connections` poll (now live in direct mode too — see
/// [`direct_list_connections`]) is what ultimately surfaces the new
/// row to the UI.
pub(super) async fn direct_authorize(
    direct: &Arc<crate::openhuman::tools::ComposioTool>,
    toolkit: &str,
    entity_id: &str,
) -> anyhow::Result<ComposioAuthorizeResponse> {
    let toolkit = toolkit.trim();
    if toolkit.is_empty() {
        anyhow::bail!("composio direct authorize: toolkit must not be empty");
    }
    let entity_id = entity_id.trim();
    let entity_id = if entity_id.is_empty() {
        "default"
    } else {
        entity_id
    };
    tracing::debug!(
        toolkit = %toolkit,
        entity_id = %entity_id,
        "[composio-direct] authorize: requesting hosted connect URL"
    );
    let connect_url = direct
        .get_connection_url(Some(toolkit), None, entity_id)
        .await?;
    tracing::debug!(
        toolkit = %toolkit,
        url_len = connect_url.len(),
        "[composio-direct] authorize: got connect url (redacted)"
    );
    Ok(ComposioAuthorizeResponse {
        connect_url,
        // No stable connection id in the v3 link response — see fn-level
        // doc. The frontend uses `connectUrl` to open the browser and
        // `listConnections` polling to detect the resulting row.
        connection_id: String::new(),
    })
}

/// Direct-mode counterpart to [`ComposioClient::execute_tool`]. Mirrors
/// the v3 `/tools/{slug}/execute` envelope into [`ComposioExecuteResponse`]
/// so the caller doesn't branch on mode for the
/// `ComposioActionExecuted` event-bus payload or the
/// markdown-vs-JSON-body preference.
///
/// Direct mode runs without the backend's billing margin, so `cost_usd`
/// is reported as `0.0`. The backend's `markdownFormatted` field is
/// likewise specific to the backend-proxied path and remains `None` for
/// direct callers, which fall back to the raw JSON envelope.
pub async fn direct_execute(
    direct: &Arc<crate::openhuman::tools::ComposioTool>,
    tool: &str,
    arguments: Option<serde_json::Value>,
    entity_id: &str,
    connection_id: Option<&str>,
) -> anyhow::Result<ComposioExecuteResponse> {
    let tool = tool.trim();
    if tool.is_empty() {
        anyhow::bail!("composio direct_execute: tool slug must not be empty");
    }
    let params = arguments.unwrap_or_else(|| serde_json::Value::Object(Default::default()));
    let entity_id = entity_id.trim();
    let entity_id_opt = (!entity_id.is_empty()).then_some(entity_id);
    let conn_id = connection_id.map(str::trim).filter(|s| !s.is_empty());
    tracing::debug!(
        tool = %tool,
        has_entity = entity_id_opt.is_some(),
        connection_id = ?conn_id,
        "[composio-direct] execute: invoking v3 /tools/{{slug}}/execute"
    );
    let raw = direct
        .execute_action(tool, params, entity_id_opt, conn_id)
        .await?;
    // v3 surfaces `successful` + `data` + `error` at the top level. If
    // none are present, treat the call as success so callers see the
    // raw payload instead of an empty error envelope.
    let successful = raw
        .get("successful")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| raw.get("success").and_then(serde_json::Value::as_bool))
        .unwrap_or(true);
    let error = raw
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let data = raw.get("data").cloned().unwrap_or(raw);
    Ok(ComposioExecuteResponse {
        data,
        successful,
        error,
        cost_usd: 0.0,
        markdown_formatted: None,
    })
}

/// Direct-mode counterpart to [`ComposioClient::list_connections`].
///
/// Calls Composio v3 `/connected_accounts` (via
/// [`crate::openhuman::tools::ComposioTool::list_connected_accounts`])
/// and maps each item to the canonical [`ComposioConnection`] so the
/// existing frontend type contract and the 5 s UI poll keep working
/// unchanged.
///
/// Toolkit slug, status, and `created_at` are extracted defensively —
/// missing or unparseable fields fall back to empty strings / `None`
/// rather than dropping the row. The status filter applied downstream
/// (`ComposioConnection::is_active`) treats empty status as inactive,
/// so a malformed row will simply not be presented as connected — the
/// fail-safe shape the user expects.
pub async fn direct_list_connections(
    direct: &Arc<crate::openhuman::tools::ComposioTool>,
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

    let items = match direct.list_connected_accounts().await {
        Ok(items) => {
            direct_auth::record_direct_auth_success(key_id);
            items
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
    let connections: Vec<ComposioConnection> = items
        .into_iter()
        .filter_map(|item| {
            let id = item.id.trim().to_string();
            if id.is_empty() {
                return None;
            }
            let toolkit = item.toolkit_slug().unwrap_or_default();
            let status = item.status.clone().unwrap_or_default();
            Some(ComposioConnection {
                id,
                toolkit,
                status,
                created_at: item.created_at.clone(),
                // Identity fields are populated by
                // `enrich_connections_with_identity` in ops.rs after
                // the full list is fetched, using cached profile data.
                account_email: None,
                workspace: None,
                username: None,
            })
        })
        .collect();
    tracing::debug!(
        count = connections.len(),
        "[composio-direct] list_connections: mapped v3 connected accounts"
    );
    Ok(ComposioConnectionsResponse { connections })
}

/// Direct-mode counterpart to [`ComposioClient::list_tools`]. Calls
/// Composio v3 `/tools?toolkits=<csv>&tags=<a>&tags=<b>` via
/// [`crate::openhuman::tools::ComposioTool::list_tool_schemas_v3`] and
/// reshapes each item into the same [`ComposioToolSchema`] envelope the
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
/// [`super::ops::should_forward_tags`] before passing `tags` here.
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
    direct: &Arc<crate::openhuman::tools::ComposioTool>,
    toolkits: &[String],
    tags: Option<&[String]>,
) -> anyhow::Result<ComposioToolsResponse> {
    let toolkit_refs: Vec<&str> = toolkits.iter().map(|s| s.as_str()).collect();
    let tag_refs: Option<Vec<&str>> = tags.map(|t| t.iter().map(|s| s.as_str()).collect());
    tracing::debug!(
        toolkits = toolkit_refs.len(),
        tags = tag_refs.as_ref().map(Vec::len).unwrap_or(0),
        "[composio-direct] list_tools: GET v3 /tools"
    );
    let items = direct
        .list_tool_schemas_v3(&toolkit_refs, tag_refs.as_deref())
        .await?;
    let tools: Vec<super::types::ComposioToolSchema> = items
        .into_iter()
        .filter(|item| !item.slug.is_empty())
        .map(|item| super::types::ComposioToolSchema {
            kind: "function".to_string(),
            function: super::types::ComposioToolFunction {
                name: item.slug,
                description: item.description,
                parameters: item.input_parameters,
                output_parameters: item.output_parameters,
            },
        })
        .collect();
    tracing::debug!(
        count = tools.len(),
        "[composio-direct] list_tools: mapped v3 tool schemas"
    );
    Ok(ComposioToolsResponse { tools })
}
