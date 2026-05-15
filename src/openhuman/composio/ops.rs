//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Result alias used by every `composio_*` op in this module.
///
/// We deliberately return a plain `String` error instead of
/// `anyhow::Error` — the controller layer in `schemas.rs` forwards
/// these straight into the RPC envelope, and `String` keeps the shape
/// obvious at a glance.
type OpResult<T> = std::result::Result<T, String>;

use std::sync::Arc;

use super::client::{
    build_composio_client, create_composio_client, direct_authorize, direct_execute,
    direct_list_connections, direct_list_tools, ComposioClient, ComposioClientKind,
};
use super::providers::{
    get_provider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};
use super::types::{
    ComposioActiveTriggersResponse, ComposioAuthorizeResponse, ComposioAvailableTriggersResponse,
    ComposioConnectionsResponse, ComposioCreateTriggerResponse, ComposioDeleteResponse,
    ComposioDisableTriggerResponse, ComposioEnableTriggerResponse, ComposioExecuteResponse,
    ComposioGithubReposResponse, ComposioToolkitsResponse, ComposioToolsResponse,
    ComposioTriggerHistoryResult,
};

/// Resolve a backend-mode [`ComposioClient`] from the root config, or
/// return an error string that the caller can surface over RPC.
///
/// Used by the **backend-only** Composio ops — `delete_connection`,
/// `list_github_repos`, the `triggers/*` family, and the provider
/// dispatch paths (`get_user_profile`, `refresh_all_identities`,
/// `sync`). These rely on the backend's bookkeeping
/// (HMAC-verified trigger fan-out, per-user provider registry, GitHub
/// repo enumeration) that the direct-mode v3 surface does not provide,
/// so they intentionally remain backend-only for now. The "mode-aware"
/// `composio_authorize` / `composio_execute` / `composio_list_*`
/// handlers go through [`create_composio_client`] instead so the
/// `config.composio.mode` toggle is honoured per call (#1710).
fn resolve_client(config: &Config) -> OpResult<ComposioClient> {
    build_composio_client(config).ok_or_else(|| {
        "composio unavailable: no backend session token. Sign in first \
         (auth_store_session)."
            .to_string()
    })
}

/// Defense-in-depth Sentry funnel for composio op-layer errors.
///
/// The shared [`crate::openhuman::integrations::IntegrationClient`]
/// (which fronts every `client.list_*` / `client.execute_tool` /
/// `client.authorize` call) already reports its own failures under
/// `domain="integrations"` with `failure="non_2xx" | "transport"` tags,
/// and the Sentry `before_send` filter (`is_transient_integrations_failure`)
/// drops the transient subset. This helper re-classifies the same
/// anyhow chain at the **op layer** under `domain="composio"` so:
///
/// 1. Future call sites that bypass `IntegrationClient` (the existing
///    `raw_delete` path, or any new bespoke HTTP client added under
///    `composio/`) still funnel through the same classifier.
/// 2. Op-layer-specific failures — provider sync errors, history archive
///    errors, profile-resolution errors — get tagged consistently rather
///    than reaching Sentry as bare `Err(String)` returned via RPC.
///
/// The classifier (`expected_error_kind`) is purely message-substring
/// based — `Backend returned 502 …`, `error sending request for url …`,
/// `operation timed out` etc. all resolve to a warn/info breadcrumb
/// without a Sentry event. Genuine bugs (404s, 500s with bug-shape
/// payloads, envelope errors) still surface.
///
/// `failure="non_2xx"` is the default tag because that is the dominant
/// shape in the leak set (OPENHUMAN-TAURI-35 / -2H: backend 502 from
/// `Backend returned …`). When the message contains a recognized
/// transport phrase (`operation timed out`, `connection refused`, `tls
/// handshake eof`, …), we tag `failure="transport"` instead so the
/// `before_send` filter's transport-phrase branch fires — and keep the
/// status tag absent (transport failures don't carry a status).
fn report_composio_op_error<E: std::fmt::Display + ?Sized>(operation: &str, err: &E) {
    // `{err:#}` renders the full anyhow chain when applicable; for plain
    // `String` / `&str` errors it falls back to the Display impl.
    let rendered = format!("{err:#}");
    let failure_tag = classify_composio_failure_tag(rendered.as_str());
    if failure_tag == "non_2xx" {
        if let Some(status) = extract_backend_returned_status(&rendered) {
            crate::core::observability::report_error_or_expected(
                rendered.as_str(),
                "composio",
                operation,
                &[("failure", failure_tag), ("status", status.as_str())],
            );
            return;
        }
    }
    crate::core::observability::report_error_or_expected(
        rendered.as_str(),
        "composio",
        operation,
        &[("failure", failure_tag)],
    );
}

/// Pick the `failure` tag for a composio op-layer error message based on
/// shape inspection. Transport-level reqwest chains (timeout, connection
/// reset, TLS handshake EOF, "error sending request for url") tag as
/// `"transport"` so the `before_send` filter's transport-phrase branch
/// fires; everything else (the dominant `Backend returned <status> …`
/// shape from the integrations layer) tags as `"non_2xx"`.
///
/// Extracted so tests can pin the routing without a Sentry test client.
fn classify_composio_failure_tag(rendered: &str) -> &'static str {
    let lower = rendered.to_ascii_lowercase();
    // `rendered`: pass to callee-normalised checks
    //   (`contains_transient_transport_phrase` handles casing internally).
    // `lower`: pre-lowered copy reused for literal substring matches that
    //   intentionally do their own case-folding here.
    // A future contributor adding a new condition should extend the side
    // that matches the new check's normaliser contract.
    let is_transport = crate::core::observability::contains_transient_transport_phrase(rendered)
        || lower.contains("error sending request");
    if is_transport {
        "transport"
    } else {
        "non_2xx"
    }
}

/// Extract the HTTP status code from a `Backend returned <status> ...`
/// rendering produced by the integrations layer. Returns `None` when no
/// numeric status follows the anchor phrase (e.g. envelope-only errors).
///
/// Surfacing the status as a Sentry tag gives the `before_send` filter's
/// transient-status branch (`is_transient_integrations_failure`) a precise
/// signal to drop the dominant 5xx leak shape (OPENHUMAN-TAURI-35 / -2H)
/// without also dropping genuine 4xx bug-shape failures that share the
/// `failure="non_2xx"` tag.
fn extract_backend_returned_status(rendered: &str) -> Option<String> {
    let lower = rendered.to_ascii_lowercase();
    let rest = lower.split_once("backend returned ")?.1;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty()).then_some(digits)
}

// ── Toolkits ────────────────────────────────────────────────────────

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");
    // Route through the mode-aware factory so direct-mode users do NOT
    // silently fall through to the backend tinyhumans tenant's allowlist.
    // [composio-direct] In direct mode we don't expose a toolkit
    // allowlist at all — the user's personal Composio account governs
    // what's available. Returning an empty list signals "no curated
    // allowlist" to the UI and prompt-builder, which matches the
    // sovereign expectation: Direct mode users manage their toolkits
    // through app.composio.dev directly.
    let kind =
        create_composio_client(config).map_err(|e| format!("[composio] list_toolkits: {e}"))?;
    match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_toolkits: backend variant");
            let resp = client.list_toolkits().await.map_err(|e| {
                report_composio_op_error("list_toolkits", &e);
                format!("[composio] list_toolkits failed: {e:#}")
            })?;
            let count = resp.toolkits.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: {count} toolkit(s) enabled")],
            ))
        }
        ComposioClientKind::Direct(_) => {
            tracing::info!(
                "[composio-direct] list_toolkits: direct mode active — no \
                 server-side allowlist is enforced; returning empty toolkits \
                 list. Users manage available toolkits via app.composio.dev."
            );
            Ok(RpcOutcome::new(
                ComposioToolkitsResponse::default(),
                vec!["composio: direct mode — no curated allowlist (toolkits \
                     managed via app.composio.dev)"
                    .to_string()],
            ))
        }
    }
}

// ── Connections ─────────────────────────────────────────────────────

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    // Route through the mode-aware factory so direct-mode users do NOT
    // accidentally see the tinyhumans-tenant connections from the
    // backend-proxied path. Mixing the two tenants is the bug behind the
    // user-reported "I switched to Direct and my old integrations are
    // still showing" symptom (#1710).
    let kind =
        create_composio_client(config).map_err(|e| format!("[composio] list_connections: {e}"))?;
    let client = match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_connections: backend variant");
            client
        }
        ComposioClientKind::Direct(direct) => {
            // [composio-direct] Translate the user's Composio v3
            // `/connected_accounts` view into the same
            // `ComposioConnectionsResponse` shape the backend-proxied
            // path emits. This is what unlocks end-to-end OAuth in
            // direct mode: once the user completes the Composio-hosted
            // flow, the UI's 5 s `composio_list_connections` poll picks
            // up the new ACTIVE row from THEIR tenant (not the
            // tinyhumans tenant) and flips the Settings badge to
            // Connected (#1710).
            tracing::info!(
                "[composio-direct] list_connections: fetching v3 \
                 /connected_accounts for the user's personal Composio tenant"
            );
            let resp = direct_list_connections(&direct)
                .await
                .map_err(|e| format!("[composio-direct] list_connections failed: {e:#}"))?;
            let active = resp.connections.iter().filter(|c| c.is_active()).count();
            let total = resp.connections.len();
            // Reconcile the integrations cache against this fresh live
            // snapshot from the user's own tenant — same defensive
            // behaviour as the backend path, so the chat runtime's
            // connected-toolkits view stays in sync within one poll
            // interval.
            sync_cache_with_connections(&resp.connections);
            return Ok(RpcOutcome::new(
                resp,
                vec![format!(
                    "composio: direct mode — {total} connection(s) listed ({active} active)"
                )],
            ));
        }
    };
    let resp = client.list_connections().await.map_err(|e| {
        report_composio_op_error("list_connections", &e);
        format!("[composio] list_connections failed: {e:#}")
    })?;
    let active = resp.connections.iter().filter(|c| c.is_active()).count();
    let total = resp.connections.len();
    // Reconcile the chat-runtime integrations cache against this fresh
    // snapshot. The desktop UI polls this RPC every 5 s, so any OAuth
    // completion that lands out-of-band from the event-bus invalidation
    // path (common on Windows when `wait_for_connection_active`'s 60 s
    // timeout fires before the user finishes the hosted flow) is still
    // reflected in chat within one poll interval.
    sync_cache_with_connections(&resp.connections);
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {total} connection(s) listed ({active} active)"
        )],
    ))
}

pub async fn composio_authorize(
    config: &Config,
    toolkit: &str,
    extra_params: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioAuthorizeResponse>> {
    tracing::debug!(toolkit = %toolkit, has_extra_params = extra_params.is_some(), "[composio] rpc authorize");
    // Route through the mode-aware factory so direct-mode users get a
    // hosted Composio OAuth URL for THEIR personal tenant — not the
    // backend tinyhumans tenant's OAuth proxy (#1710). The pre-factory
    // path hard-routed through `staging-api.tinyhumans.ai`, so a user
    // toggled into Direct mode would silently complete OAuth against
    // the wrong tenant and never see the new connection in their
    // own Composio account.
    let kind = create_composio_client(config).map_err(|e| format!("[composio] authorize: {e}"))?;
    let resp = match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!(toolkit = %toolkit, "[composio] authorize: backend variant");
            client.authorize(toolkit, extra_params).await.map_err(|e| {
                report_composio_op_error("authorize", &e);
                format!("[composio] authorize failed: {e:#}")
            })?
        }
        ComposioClientKind::Direct(direct) => {
            tracing::info!(
                toolkit = %toolkit,
                "[composio-direct] authorize: routing to user's personal Composio tenant"
            );
            // [composio-direct] `extra_params` is the backend's escape
            // hatch for toolkit-specific request fields (e.g. WhatsApp
            // `waba_id`). The v3 direct endpoint takes no such surface
            // — toolkit-specific data is configured upstream on
            // app.composio.dev when the user creates the auth config.
            // We log a warning instead of failing so the WhatsApp UX
            // (which always passes a WABA id) still works for users
            // who configured the auth config correctly on Composio's
            // side.
            if extra_params.is_some() {
                tracing::warn!(
                    toolkit = %toolkit,
                    "[composio-direct] authorize: extra_params is set but direct mode does \
                     not propagate it — configure toolkit-specific fields via \
                     app.composio.dev for your auth config"
                );
            }
            direct_authorize(&direct, toolkit, &config.composio.entity_id)
                .await
                .map_err(|e| format!("[composio-direct] authorize failed: {e:#}"))?
        }
    };

    // Publish an event so any interested subscribers (e.g. UI refreshers,
    // analytics) can react to the new connection handoff.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionCreated {
            toolkit: toolkit.to_string(),
            connection_id: resp.connection_id.clone(),
            connect_url: resp.connect_url.clone(),
        },
    );

    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: authorize flow started for {toolkit}")],
    ))
}

pub async fn composio_delete_connection(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id)
        .await
        .ok();
    let resp = client.delete_connection(connection_id).await.map_err(|e| {
        report_composio_op_error("delete_connection", &e);
        format!("[composio] delete_connection failed: {e:#}")
    })?;
    if let Some(toolkit) = toolkit.as_deref() {
        let deleted =
            super::providers::profile::delete_connected_identity_facets(toolkit, connection_id);
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            facets_deleted = deleted,
            "[composio] deleted connected identity facets after connection removal"
        );
        if let Err(e) = super::providers::profile_md::remove_provider_from_profile_md(
            &config.workspace_dir,
            toolkit,
            connection_id,
        ) {
            tracing::warn!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                error = %e,
                "[composio] PROFILE.md bullet removal failed (non-fatal)"
            );
        }
    }
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionDeleted {
            toolkit: toolkit.unwrap_or_else(|| "unknown".to_string()),
            connection_id: connection_id.to_string(),
        },
    );
    // Bust the integrations cache so the next prompt reflects the removal.
    invalidate_connected_integrations_cache();
    // Eagerly warm the cache from the backend so the very next
    // `cached_active_integrations` read (typically the orchestrator's
    // next-turn refresh, or the desktop UI's 5 s
    // `composio_list_connections` poll) sees the removal immediately
    // instead of waiting for a cache-miss round trip on the hot path.
    // Symmetric with the connect-side eager warm in
    // [`super::bus::ComposioConnectionCreatedSubscriber`]. Best-effort —
    // on backend failure the UI poll repopulates within ~5 s as a
    // safety net.
    //
    // Use the status-distinguishing fetcher so we log
    // `Authoritative(empty)` and backend unavailability differently —
    // `fetch_connected_integrations` collapses both to `Vec::new()`
    // and would otherwise hide auth/backend failures from incident
    // triage.
    match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(entries) => {
            tracing::debug!(
                connection_id = %connection_id,
                cached_entries = entries.len(),
                "[composio] eagerly warmed integrations cache after connection deletion"
            );
        }
        FetchConnectedIntegrationsStatus::Unavailable => {
            tracing::warn!(
                connection_id = %connection_id,
                "[composio] eager cache warm after connection deletion skipped: backend unavailable"
            );
        }
    }
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

// ── Tools ───────────────────────────────────────────────────────────

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    tracing::debug!(?toolkits, "[composio] rpc list_tools");
    // Route through the mode-aware factory. In direct mode the backend
    // tool catalogue (which is shaped by the tinyhumans-tenant
    // allowlist + curated whitelist) does NOT apply — the user's
    // personal Composio account governs discovery via app.composio.dev.
    // Mirrors the empty-response short-circuit in `composio_list_toolkits`
    // / `composio_list_connections` so the three "list_*" surfaces
    // behave consistently and we don't accidentally leak backend-tenant
    // data into direct mode (#1710).
    let kind = create_composio_client(config).map_err(|e| format!("[composio] list_tools: {e}"))?;
    match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_tools: backend variant");
            let resp = client.list_tools(toolkits.as_deref()).await.map_err(|e| {
                report_composio_op_error("list_tools", &e);
                format!("[composio] list_tools failed: {e:#}")
            })?;
            let count = resp.tools.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: {count} tool(s) listed")],
            ))
        }
        ComposioClientKind::Direct(direct) => {
            // [composio-direct] Discovery now hits Composio v3 `/tools`
            // directly with the user's own API key. Tenant isolation is
            // preserved (we never surface backend-tenant catalogue here),
            // and the schemas Composio returns are tenant-agnostic so
            // the LLM agent gets the same model-callable shape backend
            // mode surfaces. Scope the request to the user's connected
            // toolkits when no explicit filter was supplied — keeps the
            // response bounded and skips schemas the agent can't call.
            let scope: Vec<String> = match toolkits {
                Some(list) if !list.is_empty() => list,
                _ => {
                    let conns = direct_list_connections(&direct).await.map_err(|e| {
                        format!("[composio-direct] list_tools: prefetch connections failed: {e:#}")
                    })?;
                    let mut v: Vec<String> = conns
                        .connections
                        .iter()
                        .filter(|c| c.is_active())
                        .map(|c| c.normalized_toolkit())
                        .filter(|t| !t.is_empty())
                        .collect();
                    v.sort();
                    v.dedup();
                    v
                }
            };
            if scope.is_empty() {
                tracing::info!(
                    "[composio-direct] list_tools: no connected toolkits on this tenant — \
                     returning empty tool list"
                );
                return Ok(RpcOutcome::new(
                    ComposioToolsResponse::default(),
                    vec!["composio: direct mode — 0 tool(s) listed (no connected \
                         toolkits on this tenant)"
                        .to_string()],
                ));
            }
            tracing::debug!(
                toolkits = scope.len(),
                "[composio-direct] list_tools: fetching v3 tool schemas"
            );
            let mut resp = direct_list_tools(&direct, &scope)
                .await
                .map_err(|e| format!("[composio-direct] list_tools failed: {e:#}"))?;
            // Apply the same curated-whitelist + user-scope filter the
            // backend path runs — schemas may be tenant-agnostic but
            // OpenHuman's curation policy isn't, and direct-mode users
            // should benefit from the same safety net (e.g. dangerous
            // destructive actions hidden by default).
            let before = resp.tools.len();
            filter_list_tools_response_for_direct(&mut resp).await;
            let after = resp.tools.len();
            tracing::debug!(
                before,
                after,
                dropped = before - after,
                "[composio-direct] list_tools: curated filter applied"
            );
            let count = resp.tools.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!(
                    "composio: direct mode — {count} tool(s) listed across \
                     {} toolkit(s)",
                    scope.len()
                )],
            ))
        }
    }
}

/// Apply OpenHuman's curated-whitelist + user-scope visibility filter to
/// a fresh `ComposioToolsResponse` in direct mode. Mirrors the per-call
/// filter loop in `tools.rs::filter_list_tools_response` so backend and
/// direct surfaces share the same safety net.
async fn filter_list_tools_response_for_direct(resp: &mut ComposioToolsResponse) {
    use super::providers::{
        catalog_for_toolkit, classify_unknown, find_curated, get_provider,
        load_user_scope_or_default, toolkit_from_slug,
    };

    let mut keep: Vec<bool> = Vec::with_capacity(resp.tools.len());
    for t in &resp.tools {
        let slug = &t.function.name;
        let Some(toolkit) = toolkit_from_slug(slug) else {
            keep.push(true);
            continue;
        };
        let pref = load_user_scope_or_default(&toolkit).await;
        let catalog = get_provider(&toolkit)
            .and_then(|p| p.curated_tools())
            .or_else(|| catalog_for_toolkit(&toolkit));
        let allowed = match catalog {
            Some(cat) => match find_curated(cat, slug) {
                Some(curated) => pref.allows(curated.scope),
                None => false,
            },
            None => pref.allows(classify_unknown(slug)),
        };
        keep.push(allowed);
    }
    let drained: Vec<_> = resp.tools.drain(..).collect();
    resp.tools = drained
        .into_iter()
        .zip(keep)
        .filter_map(|(tool, keep_it)| if keep_it { Some(tool) } else { None })
        .collect();
}

// ── Execute ─────────────────────────────────────────────────────────

pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, "[composio] rpc execute");
    // Route through the mode-aware factory so direct-mode users hit
    // their personal Composio tenant for tool execution. Mirrors the
    // agent-tool path's `ComposioExecuteTool::execute` (commit
    // 814fdd97); the shared `direct_execute` helper in `client.rs`
    // keeps the envelope identical between backend and direct so the
    // `ComposioActionExecuted` event-bus payload, markdown-vs-JSON
    // body preference, and cost-USD log line all stay uniform (#1710).
    let kind = create_composio_client(config).map_err(|e| format!("[composio] execute: {e}"))?;
    let started = std::time::Instant::now();
    let result = match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!(tool = %tool, "[composio] execute: backend variant");
            client.execute_tool(tool, arguments).await
        }
        ComposioClientKind::Direct(direct) => {
            tracing::debug!(tool = %tool, "[composio-direct] execute: direct variant");
            direct_execute(&direct, tool, arguments, &config.composio.entity_id).await
        }
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(resp) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: resp.successful,
                    error: resp.error.clone(),
                    cost_usd: resp.cost_usd,
                    elapsed_ms,
                },
            );
            // Backend (tinyhumansai/backend#683) now parses all composio
            // payloads server-side and returns a `markdownFormatted`
            // string for known tools, so callers should consume that
            // directly. Core no longer reshapes `resp.data` here. Memory
            // ingestion paths still call `post_process_action_result`
            // explicitly when they need the structured slim envelope.
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: executed {tool} ({elapsed_ms}ms)")],
            ))
        }
        Err(e) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            report_composio_op_error("execute", &e);
            Err(format!("[composio] execute failed: {e:#}"))
        }
    }
}

// ── GitHub repos + trigger provisioning ─────────────────────────────

pub async fn composio_list_github_repos(
    config: &Config,
    connection_id: Option<String>,
) -> OpResult<RpcOutcome<ComposioGithubReposResponse>> {
    tracing::debug!(?connection_id, "[composio] rpc list_github_repos");
    let client = resolve_client(config)?;
    let resp = client
        .list_github_repos(connection_id.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_github_repos", &e);
            format!("[composio] list_github_repos failed: {e:#}")
        })?;
    let count = resp.repositories.len();
    let connection_id = resp.connection_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {count} github repo(s) listed for connection {connection_id}"
        )],
    ))
}

pub async fn composio_create_trigger(
    config: &Config,
    slug: &str,
    connection_id: Option<String>,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioCreateTriggerResponse>> {
    tracing::debug!(slug = %slug, ?connection_id, "[composio] rpc create_trigger");
    let client = resolve_client(config)?;
    let resp = client
        .create_trigger(slug, connection_id.as_deref(), trigger_config)
        .await
        .map_err(|e| {
            report_composio_op_error("create_trigger", &e);
            format!("[composio] create_trigger failed: {e:#}")
        })?;
    let trigger_id = resp.trigger_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: trigger {trigger_id} created for slug {slug}"
        )],
    ))
}

// ── Trigger management (catalog + enable/disable) ──────────────────

pub async fn composio_list_available_triggers(
    config: &Config,
    toolkit: &str,
    connection_id: Option<String>,
) -> OpResult<RpcOutcome<ComposioAvailableTriggersResponse>> {
    tracing::debug!(toolkit = %toolkit, ?connection_id, "[composio] rpc list_available_triggers");
    let client = resolve_client(config)?;
    let resp = client
        .list_available_triggers(toolkit, connection_id.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_available_triggers", &e);
            format!("[composio] list_available_triggers failed: {e:#}")
        })?;
    let count = resp.triggers.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {count} available trigger(s) for toolkit {toolkit}"
        )],
    ))
}

pub async fn composio_list_triggers(
    config: &Config,
    toolkit: Option<String>,
) -> OpResult<RpcOutcome<ComposioActiveTriggersResponse>> {
    tracing::debug!(?toolkit, "[composio] rpc list_triggers");
    let client = resolve_client(config)?;
    let resp = client
        .list_active_triggers(toolkit.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_triggers", &e);
            format!("[composio] list_triggers failed: {e:#}")
        })?;
    let count = resp.triggers.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} active trigger(s) listed")],
    ))
}

pub async fn composio_enable_trigger(
    config: &Config,
    connection_id: &str,
    slug: &str,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioEnableTriggerResponse>> {
    tracing::debug!(slug = %slug, connection_id = %connection_id, "[composio] rpc enable_trigger");
    let client = resolve_client(config)?;
    let resp = client
        .enable_trigger(connection_id, slug, trigger_config)
        .await
        .map_err(|e| {
            report_composio_op_error("enable_trigger", &e);
            format!("[composio] enable_trigger failed: {e:#}")
        })?;
    let trigger_id = resp.trigger_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: enabled trigger {slug} → {trigger_id}")],
    ))
}

pub async fn composio_disable_trigger(
    config: &Config,
    trigger_id: &str,
) -> OpResult<RpcOutcome<ComposioDisableTriggerResponse>> {
    tracing::debug!(trigger_id = %trigger_id, "[composio] rpc disable_trigger");
    let client = resolve_client(config)?;
    let resp = client.disable_trigger(trigger_id).await.map_err(|e| {
        report_composio_op_error("disable_trigger", &e);
        format!("[composio] disable_trigger failed: {e:#}")
    })?;
    let message = if resp.deleted {
        format!("composio: disabled trigger {trigger_id}")
    } else {
        format!("composio: trigger {trigger_id} was not active")
    };
    Ok(RpcOutcome::new(resp, vec![message]))
}

// ── Trigger history ────────────────────────────────────────────────

pub async fn composio_list_trigger_history(
    config: &Config,
    limit: Option<usize>,
) -> OpResult<RpcOutcome<ComposioTriggerHistoryResult>> {
    let requested_limit = limit.unwrap_or(100).clamp(1, 500);
    let workspace_label = config
        .workspace_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("<workspace>");
    tracing::debug!(
        limit = requested_limit,
        workspace = workspace_label,
        "[composio] rpc list_trigger_history"
    );

    let store = super::trigger_history::global().ok_or_else(|| {
        "[composio] trigger history unavailable: archive store is not initialized".to_string()
    })?;

    let history = store
        .list_recent(requested_limit)
        .map_err(|error| format!("[composio] list_trigger_history failed: {error}"))?;
    let count = history.entries.len();

    Ok(RpcOutcome::new(
        history,
        vec![format!(
            "composio: {count} trigger history entrie(s) loaded (archive present)"
        )],
    ))
}

// ── Provider-backed ops ─────────────────────────────────────────────
//
// `composio_get_user_profile` and `composio_sync` route through the
// per-toolkit `ComposioProvider` registry instead of executing a
// single Composio action directly. The caller passes a `connection_id`,
// the op resolves the connection's toolkit slug from the backend, looks
// up the provider, and dispatches to it.
//
// These exist because individual toolkits need to do *several*
// `composio.execute` calls + bespoke result reshaping to produce a
// usable user profile or sync snapshot — wrapping that in a single
// RPC method keeps the UI/agent surface tiny and consistent across
// toolkits.

/// Look up the toolkit slug for an existing connection. Returns an
/// error string if the connection is unknown to the backend.
async fn resolve_toolkit_for_connection(
    client: &ComposioClient,
    connection_id: &str,
) -> OpResult<String> {
    tracing::debug!(connection_id = %connection_id, "[composio] resolve_toolkit_for_connection");
    let resp = client.list_connections().await.map_err(|e| {
        report_composio_op_error("resolve_toolkit_for_connection", &e);
        format!("[composio] list_connections failed: {e:#}")
    })?;
    let conn = resp
        .connections
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("[composio] no connection with id '{connection_id}'"))?;
    Ok(conn.toolkit)
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user
/// profile for a connected account by dispatching to the toolkit's
/// registered [`super::providers::ComposioProvider`].
pub async fn composio_get_user_profile(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ProviderUserProfile>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc get_user_profile");
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    // #1710: drop the pre-baked `client` field from `ProviderContext`.
    // The factory resolves a fresh client per `ctx.execute(...)` call so
    // a mode toggle is honoured immediately. We keep the local `client`
    // binding alive for the toolkit lookup above (which still uses the
    // explicit handle); the context itself just carries `Arc<Config>`.
    let _ = client;
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let profile = provider.fetch_user_profile(&ctx).await.map_err(|e| {
        report_composio_op_error("get_user_profile", &e);
        format!("[composio] get_user_profile({toolkit}) failed: {e}")
    })?;

    // Side-effect: persist profile fields into the local user_profile
    // facet table so any RPC call also refreshes the local store.
    let facets = provider.identity_set(&profile);
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] identity_set persisted profile facets from get_user_profile"
    );

    Ok(RpcOutcome::new(
        profile,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_refresh_all_identities` — re-fetch the user
/// profile for every active connection and persist via `identity_set`.
/// Used to populate kind-tagged `user_profile` rows on existing
/// connections after the #1365 schema rewrite without waiting for the
/// next periodic sync tick.
///
/// Best-effort per connection: a failure on one toolkit does not abort
/// the others. Returns aggregate counts plus a per-connection trail in
/// the envelope messages.
pub async fn composio_refresh_all_identities(
    config: &Config,
) -> OpResult<RpcOutcome<RefreshIdentitiesReport>> {
    tracing::info!("[composio] rpc refresh_all_identities");
    let client = resolve_client(config)?;
    let conns = client.list_connections().await.map_err(|e| {
        report_composio_op_error("refresh_all_identities", &e);
        format!("[composio] list_connections failed: {e:#}")
    })?;

    let mut report = RefreshIdentitiesReport::default();
    let mut messages: Vec<String> = Vec::with_capacity(conns.connections.len() + 1);

    for conn in conns.connections {
        if !conn.is_active() {
            report.skipped_inactive += 1;
            continue;
        }
        let toolkit = conn.toolkit.clone();
        let connection_id = conn.id.clone();

        let Some(provider) = get_provider(&toolkit) else {
            tracing::debug!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                "[composio] refresh_all_identities: no native provider — skipping"
            );
            report.skipped_no_provider += 1;
            messages.push(format!(
                "{toolkit}/{connection_id}: skipped (no native provider)"
            ));
            continue;
        };

        let ctx = ProviderContext {
            config: Arc::new(config.clone()),
            toolkit: toolkit.clone(),
            connection_id: Some(connection_id.clone()),
        };

        match provider.fetch_user_profile(&ctx).await {
            Ok(profile) => {
                let rows = provider.identity_set(&profile);
                report.refreshed += 1;
                report.rows_written += rows;
                tracing::debug!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    rows_written = rows,
                    "[composio] refresh_all_identities: identity_set ok"
                );
                messages.push(format!("{toolkit}/{connection_id}: {rows} row(s)"));
            }
            Err(e) => {
                report.failed += 1;
                tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    error = %e,
                    "[composio] refresh_all_identities: fetch_user_profile failed"
                );
                messages.push(format!("{toolkit}/{connection_id}: ERROR — {e}"));
            }
        }
    }

    let summary = format!(
        "composio: refreshed {ok}/{tried} active conn(s) — {rows} rows; \
         {fail} failed, {nopv} skipped (no provider), {inact} inactive",
        ok = report.refreshed,
        // `tried` is the count of active connections we actually scanned —
        // include `skipped_no_provider` so the denominator covers the full
        // active set, not just provider-backed ones (#1381 review).
        tried = report.refreshed + report.failed + report.skipped_no_provider,
        rows = report.rows_written,
        fail = report.failed,
        nopv = report.skipped_no_provider,
        inact = report.skipped_inactive,
    );
    let mut envelope = vec![summary];
    envelope.extend(messages);
    Ok(RpcOutcome::new(report, envelope))
}

/// Aggregate result of [`composio_refresh_all_identities`].
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshIdentitiesReport {
    pub refreshed: usize,
    pub failed: usize,
    pub skipped_no_provider: usize,
    pub skipped_inactive: usize,
    pub rows_written: usize,
}

/// `openhuman.composio_sync` — run a sync pass for a connected account
/// by dispatching to the toolkit's registered provider. `reason` is
/// `"manual"` by default; the periodic scheduler passes `"periodic"`
/// and the OAuth event subscriber passes `"connection_created"`.
pub async fn composio_sync(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
) -> OpResult<RpcOutcome<SyncOutcome>> {
    let reason = parse_sync_reason(reason.as_deref())?;
    tracing::debug!(
        connection_id = %connection_id,
        reason = reason.as_str(),
        "[composio] rpc sync"
    );
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    let _ = client; // see analogous comment above — drop the pre-baked client (#1710).
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let outcome = provider.sync(&ctx, reason).await.map_err(|e| {
        report_composio_op_error("sync", &e);
        format!("[composio] sync({toolkit}) failed: {e}")
    })?;

    let summary = outcome.summary.clone();
    Ok(RpcOutcome::new(outcome, vec![summary]))
}

/// Parse the optional `reason` parameter into a [`SyncReason`].
///
/// `None` and the explicit `"manual"` value both map to
/// [`SyncReason::Manual`]. Any other unrecognized string is rejected
/// with a clear error so a typo in a caller (UI, CLI, agent) surfaces
/// at the RPC boundary instead of being silently coerced.
fn parse_sync_reason(raw: Option<&str>) -> OpResult<SyncReason> {
    match raw {
        None | Some("manual") => Ok(SyncReason::Manual),
        Some("periodic") => Ok(SyncReason::Periodic),
        Some("connection_created") => Ok(SyncReason::ConnectionCreated),
        Some(other) => Err(format!(
            "[composio] unrecognized sync reason '{other}': expected one of \
             'manual', 'periodic', 'connection_created'"
        )),
    }
}

// ── Prompt integration discovery ────────────────────────────────────

use crate::openhuman::context::prompt::{ConnectedIntegration, ConnectedIntegrationTool};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// Defensive TTL on the integrations cache.
///
/// Background: the primary invalidation path is the
/// `ComposioConnectionCreated` → `wait_for_connection_active` bus flow
/// (see [`super::bus::ComposioConnectionCreatedSubscriber`]), which
/// polls the backend for up to 60 s after `composio_authorize` returns
/// a `connectUrl`. On Windows the OAuth round-trip can exceed that
/// window (Defender SmartScreen, slower browser launch, extra consent
/// dialogs), so the invalidation call never fires and the chat
/// runtime's cache stays frozen on the pre-connect snapshot even
/// though the Settings UI polls `composio_list_connections` every 5 s
/// and shows the user as "Connected".
///
/// The cross-platform defenses we layer on top:
///   1. [`composio_list_connections`] diff-invalidates the cache whenever
///      the backend's active-toolkit set diverges from what's cached,
///      so a running UI keeps the chat cache in sync within one poll
///      interval.
///   2. This TTL caps worst-case staleness at 60 s regardless of
///      whether the UI is open, the bus fires, or the user reconnected
///      out-of-band.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Cached entry: the integrations list plus the timestamp we wrote it.
#[derive(Clone)]
struct CachedIntegrations {
    entries: Vec<ConnectedIntegration>,
    cached_at: Instant,
}

/// Process-wide cache for connected integrations, keyed by the config
/// identity (the `config_path` string) so different user contexts don't
/// collide. Each entry is populated on first fetch and returned on
/// subsequent calls until explicitly invalidated or the TTL expires.
static INTEGRATIONS_CACHE: LazyLock<RwLock<HashMap<String, CachedIntegrations>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Derive a stable cache key from a [`Config`]. We use the stringified
/// `config_path` because it uniquely identifies a user context (it
/// resolves to the per-user openhuman dir).
fn cache_key(config: &Config) -> String {
    config.config_path.display().to_string()
}

/// Clear cached connected integrations so the next call to
/// [`fetch_connected_integrations`] hits the backend again.
///
/// Called by [`super::bus::ComposioConnectionCreatedSubscriber`] when a
/// new OAuth connection completes, by [`composio_list_connections`]
/// when it observes a divergence between the backend response and the
/// cached snapshot, and from tests. Clears the entire map because the
/// callers don't carry a config reference.
pub fn invalidate_connected_integrations_cache() {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        let entries = guard.len();
        guard.clear();
        tracing::info!(
            cached_keys = entries,
            "[composio][integrations] cache invalidated"
        );
    }
}

/// Read-only snapshot of the currently cached connected integrations for
/// the given config, or [`None`] when the cache is empty, expired, or
/// the lock is held by a writer.
///
/// Designed for hot-path callers that want a cheap "what does the cache
/// already say?" probe without triggering a backend fetch. The agent
/// harness uses this on every turn to detect mid-session connection
/// changes — it relies on the desktop UI's 5 s `composio_list_connections`
/// poll (which calls into [`fetch_connected_integrations`] and
/// repopulates this cache) plus the event-driven invalidation path to
/// keep the cache current.
///
/// `try_read` (not `read`) so a writer in progress — e.g. the UI poll
/// repopulating the cache — never blocks a turn. Worst case the agent
/// sees `None` for one turn while the writer holds the lock; the next
/// turn picks up the value naturally.
///
/// TTL is enforced defensively: entries older than [`CACHE_TTL`] are
/// treated as missing even though they're still in the map (a stale
/// entry would otherwise pin the agent to a frozen view if every
/// invalidation path silently failed).
pub fn cached_active_integrations(config: &Config) -> Option<Vec<ConnectedIntegration>> {
    let key = cache_key(config);
    let guard = match INTEGRATIONS_CACHE.try_read() {
        Ok(g) => g,
        Err(_) => {
            tracing::trace!(
                key = %key,
                "[composio][integrations_cache] cached_active_integrations:lock_contended"
            );
            return None;
        }
    };
    let Some(cached) = guard.get(&key) else {
        tracing::trace!(
            key = %key,
            "[composio][integrations_cache] cached_active_integrations:miss"
        );
        return None;
    };
    let age = cached.cached_at.elapsed();
    if age > CACHE_TTL {
        tracing::trace!(
            key = %key,
            age_ms = age.as_millis() as u64,
            ttl_ms = CACHE_TTL.as_millis() as u64,
            "[composio][integrations_cache] cached_active_integrations:expired"
        );
        return None;
    }
    tracing::trace!(
        key = %key,
        entries = cached.entries.len(),
        age_ms = age.as_millis() as u64,
        "[composio][integrations_cache] cached_active_integrations:hit"
    );
    Some(cached.entries.clone())
}

/// Stable hash of the *routing-relevant* slice of a connected-integrations
/// snapshot.
///
/// Two snapshots produce the same hash iff they would synthesise the
/// same `delegate_<toolkit>` tool set in the orchestrator's
/// function-calling schema. The hash is:
///
///   - **Order-independent** — callers don't need to sort the input.
///   - **Description-insensitive** — Composio catalogue text edits don't
///     trigger a refresh. The schema's tool-description field still
///     picks up new text on the next *real* (membership-changing)
///     refresh, so descriptions are never permanently stale.
///   - **Process-local** — [`std::collections::hash_map::DefaultHasher`]
///     is randomly seeded per process. Fine because we only compare
///     hashes within one process lifetime.
///
/// Only `connected == true` entries contribute. Unconnected toolkits are
/// stripped by [`super::super::tools::orchestrator_tools::collect_orchestrator_tools`]
/// anyway, so churn among the unconnected set never changes the agent's
/// surface and shouldn't trigger a refresh.
pub fn connected_set_hash(integrations: &[ConnectedIntegration]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut slugs: Vec<&str> = integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| i.toolkit.as_str())
        .collect();
    slugs.sort();

    let mut hasher = DefaultHasher::new();
    slugs.hash(&mut hasher);
    hasher.finish()
}

/// Collect the set of toolkit slugs marked `connected` in a snapshot.
///
/// Exposed to [`sync_cache_with_connections`] so it can diff the live
/// backend connection list against what the chat runtime currently
/// believes is connected.
fn connected_toolkit_set(integrations: &[ConnectedIntegration]) -> HashSet<String> {
    integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| i.toolkit.clone())
        .collect()
}

/// Reconcile the process-wide integrations cache with a fresh backend
/// `list_connections` response.
///
/// Called from [`composio_list_connections`], which the desktop UI
/// polls every 5 s (see `app/src/lib/composio/hooks.ts`). When the set
/// of ACTIVE/CONNECTED toolkits in the response differs from what's in
/// the cache, we invalidate so the chat runtime re-fetches on its next
/// `fetch_connected_integrations` call. This keeps tool availability
/// in chat in sync with the badge the user sees in Settings, even when
/// the primary event-bus invalidation path misses (e.g. Windows OAuth
/// flows that overrun the 60 s readiness poll).
fn sync_cache_with_connections(connections: &[super::types::ComposioConnection]) {
    let live_active: HashSet<String> = connections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.normalized_toolkit())
        .filter(|toolkit| !toolkit.is_empty())
        .collect();

    // Read once to decide whether any cache entry is out of sync. We
    // clone out the keys + connected sets so we can release the read
    // lock before taking the write lock.
    let divergent_keys: Vec<(String, HashSet<String>, HashSet<String>)> = {
        let Ok(guard) = INTEGRATIONS_CACHE.read() else {
            return;
        };
        guard
            .iter()
            .filter_map(|(key, cached)| {
                let cached_set = connected_toolkit_set(&cached.entries);
                if cached_set != live_active {
                    Some((key.clone(), cached_set, live_active.clone()))
                } else {
                    None
                }
            })
            .collect()
    };

    if divergent_keys.is_empty() {
        tracing::debug!(
            live_connected = live_active.len(),
            "[composio][integrations] list_connections matches cache — no invalidation needed"
        );
        return;
    }

    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        for (key, cached_set, live_set) in divergent_keys {
            // Diff logging — makes Windows-timing regressions easy to
            // catch in user-supplied debug dumps without leaking any
            // PII (toolkit slugs are public strings like "gmail").
            let added: Vec<&String> = live_set.difference(&cached_set).collect();
            let removed: Vec<&String> = cached_set.difference(&live_set).collect();
            tracing::info!(
                key = %key,
                ?added,
                ?removed,
                "[composio][integrations] cache diverges from backend — invalidating"
            );
            guard.remove(&key);
        }
    }
}

/// Fetch the user's active Composio connections and their available
/// tool actions, returning a prompt-ready summary.
///
/// This is the **single source of truth** for connected integration
/// data injected into system prompts — both the agent turn loop and
/// the debug dump CLI call this function.
///
/// Results are cached process-wide (keyed by config identity) and
/// returned instantly on subsequent calls. The cache is invalidated
/// when a new connection is created
/// (via [`invalidate_connected_integrations_cache`]), when a UI
/// `list_connections` poll observes a divergent live set, when
/// [`CACHE_TTL`] expires, or on process restart.
///
/// Best-effort: returns an empty vec when the user isn't signed in,
/// the backend is unreachable, or any step fails.
pub async fn fetch_connected_integrations(config: &Config) -> Vec<ConnectedIntegration> {
    match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(v) => v,
        FetchConnectedIntegrationsStatus::Unavailable => Vec::new(),
    }
}

/// Discriminated outcome from [`fetch_connected_integrations_status`].
///
/// Lets callers distinguish "the backend confirmed the user has zero
/// active connections right now" from "we couldn't talk to the backend
/// (no client, transient failure, …) and have no truth to report".
///
/// The legacy [`fetch_connected_integrations`] collapses both into an
/// empty `Vec`, which is fine for prompt-building (they look the same)
/// but dangerous for spawn-time allowlist gates — using empty as truth
/// in the unavailable case would silently wipe the user's allowlist
/// during a transient 5xx.
#[derive(Debug, Clone)]
pub enum FetchConnectedIntegrationsStatus {
    /// Backend was reachable. Vec may legitimately be empty (no
    /// allowlisted toolkits, or no active connections).
    Authoritative(Vec<ConnectedIntegration>),
    /// Backend wasn't reachable (no auth client, transient error). The
    /// caller should fall back to its prior snapshot rather than treat
    /// "no connections" as truth.
    Unavailable,
}

/// Status-returning variant of [`fetch_connected_integrations`].
///
/// Same caching, same cache-invalidation semantics — only the return
/// shape differs. Cache hits are by definition `Authoritative` because
/// we only cache the `Some(...)` arm of `_uncached` (i.e. results the
/// backend confirmed).
pub async fn fetch_connected_integrations_status(
    config: &Config,
) -> FetchConnectedIntegrationsStatus {
    let key = cache_key(config);

    // Fast path: return cached result if fresh. Stale entries fall
    // through to the backend fetch below so the chat runtime can never
    // be more than `CACHE_TTL` behind a real-world change.
    if let Ok(guard) = INTEGRATIONS_CACHE.read() {
        if let Some(cached) = guard.get(&key) {
            let age = cached.cached_at.elapsed();
            if age < CACHE_TTL {
                tracing::debug!(
                    count = cached.entries.len(),
                    age_ms = age.as_millis() as u64,
                    key = %key,
                    "[composio][integrations] returning cached result"
                );
                return FetchConnectedIntegrationsStatus::Authoritative(cached.entries.clone());
            }
            tracing::info!(
                count = cached.entries.len(),
                age_ms = age.as_millis() as u64,
                ttl_ms = CACHE_TTL.as_millis() as u64,
                key = %key,
                "[composio][integrations] cache entry expired — refetching"
            );
        }
    }

    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
                guard.insert(
                    key,
                    CachedIntegrations {
                        entries: result.clone(),
                        cached_at: Instant::now(),
                    },
                );
            }
            FetchConnectedIntegrationsStatus::Authoritative(result)
        }
        None => {
            // No auth / client unavailable — do NOT cache so a
            // subsequent call with a different config can retry.
            FetchConnectedIntegrationsStatus::Unavailable
        }
    }
}

/// The actual backend fetch, called on cache miss.
///
/// Returns `Some(vec)` when the backend was reachable. The returned
/// vector is the merged **integration overview** — every toolkit in
/// the backend allowlist appears as one entry, with a `connected`
/// flag indicating whether the user has an active OAuth connection.
/// Connected entries also carry the per-action tool catalogue
/// (fetched in a single batched call).
///
/// Returns `None` when we couldn't even build a client (no auth),
/// signalling the caller should NOT cache this result.
async fn fetch_connected_integrations_uncached(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    use super::client::{create_composio_client, direct_list_connections, ComposioClientKind};
    use super::providers::toolkit_description;

    // Route via the mode-aware factory so the chat-agent's
    // "connected_integrations" view reflects the live tenant — backend
    // (tinyhumans) or direct (user's personal Composio). Prior to #1710
    // Wave 3 this path called `build_composio_client` directly, which
    // is backend-only — after a `composio.mode = "direct"` toggle the
    // cache kept replaying the tinyhumans-tenant connections back into
    // the integration overview (e.g. gmail / notion appearing as
    // connected in direct mode even when the user's direct tenant had
    // a different set of toolkits). Resolving per call closes the
    // loop: `ComposioConfigChangedSubscriber` invalidates the cache on
    // toggle and the next miss re-populates it from the live tenant.
    let kind = match create_composio_client(config) {
        Ok(kind) => kind,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "[composio] fetch_connected_integrations: no client (not signed in?)"
            );
            return None;
        }
    };

    // Pull the allowlist + connections + tool catalogue. Backend mode
    // walks the tinyhumans tenant's curated allowlist via
    // `list_toolkits`; direct mode has no centralised allowlist (per
    // `ops::composio_list_toolkits`'s direct-mode branch) so the
    // user's set of active connections IS the universe of valid
    // toolkit arguments.
    //
    // On transient errors we return `None` instead of a degraded
    // `Some(Vec::new())` so `fetch_connected_integrations` does NOT
    // cache the failure. Caching an empty allowlist would hide every
    // integration from the orchestrator until the process restarts or
    // the cache is explicitly invalidated — a single 5xx during
    // startup would silently break delegation for the whole session.
    let (allowlisted_toolkits, connections, tools_by_toolkit): (
        Vec<String>,
        Vec<super::types::ComposioConnection>,
        Vec<super::types::ComposioToolSchema>,
    ) = match &kind {
        ComposioClientKind::Backend(client) => {
            let allowlist: Vec<String> = match client.list_toolkits().await {
                Ok(resp) => resp
                    .toolkits
                    .into_iter()
                    .map(|toolkit| toolkit.trim().to_ascii_lowercase())
                    .filter(|toolkit| !toolkit.is_empty())
                    .collect(),
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_toolkits (backend) failed: {e}"
                    );
                    return None;
                }
            };

            if allowlist.is_empty() {
                tracing::debug!(
                    "[composio] fetch_connected_integrations: backend allowlist is empty"
                );
                return Some(Vec::new());
            }

            let connections = match client.list_connections().await {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (backend) failed: {e}"
                    );
                    return None;
                }
            };

            // Tool catalogue scoped to the active subset only —
            // not-connected toolkits won't be invoked from a sub-agent.
            let connected_slugs_for_tools: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            let tools = if connected_slugs_for_tools.is_empty() {
                Vec::new()
            } else {
                match client.list_tools(Some(&connected_slugs_for_tools)).await {
                    Ok(resp) => resp.tools,
                    Err(e) => {
                        tracing::warn!(
                            "[composio] fetch_connected_integrations: list_tools (backend) failed: {e}"
                        );
                        return None;
                    }
                }
            };

            (allowlist, connections, tools)
        }
        ComposioClientKind::Direct(direct) => {
            // Direct mode: walk the user's personal Composio tenant
            // for *connection state* (active accounts on their key) —
            // there's no central allowlist in direct mode, so the
            // active set IS the allowlist.
            //
            // Tool *schemas* are tenant-agnostic — Composio's action
            // definitions (e.g. GMAIL_SEND_EMAIL parameter shape) are
            // identical regardless of which Composio tenant a user is
            // connected via. So we best-effort fetch schemas through
            // the backend client (curated list_tools) if a backend
            // session is available, even though connection routing
            // goes to the user's direct tenant. This preserves the
            // chat agent's "21 gmail actions available" view while
            // execution itself (via `ComposioActionTool` / Wave 1
            // factory) still routes to the user's tenant. Direct-only
            // users without a backend session get empty tools — that
            // matches `composio_list_tools`'s direct-mode policy and
            // the `subagent_runner` LazyToolkitResolver still resolves
            // tools lazily at delegation time.
            let connections = match direct_list_connections(direct).await {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (direct) failed: {e:#}"
                    );
                    return None;
                }
            };
            let allowlist: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            if allowlist.is_empty() {
                tracing::info!(
                    "[composio-direct] fetch_connected_integrations: direct tenant has no active connections; returning empty overview"
                );
                return Some(Vec::new());
            }
            tracing::debug!(
                connected = allowlist.len(),
                "[composio-direct] fetch_connected_integrations: using direct tenant's active set as allowlist (no central allowlist in direct mode)"
            );

            // Best-effort: pull tool schemas via the backend client
            // (definitional source). Failure is non-fatal — we fall
            // back to empty tools and let lazy resolution handle it.
            let tools = match super::client::build_composio_client(config) {
                Some(backend_client) => match backend_client.list_tools(Some(&allowlist)).await {
                    Ok(resp) => {
                        tracing::debug!(
                            count = resp.tools.len(),
                            "[composio-direct] fetch_connected_integrations: pulled tool schemas from backend (tenant-agnostic definitional source)"
                        );
                        resp.tools
                    }
                    Err(e) => {
                        tracing::info!(
                            "[composio-direct] fetch_connected_integrations: backend list_tools failed (will use lazy fallback at delegation time): {e:#}"
                        );
                        Vec::new()
                    }
                },
                None => {
                    tracing::info!(
                        "[composio-direct] fetch_connected_integrations: no backend session for schema fetch; lazy fallback at delegation time"
                    );
                    Vec::new()
                }
            };
            (allowlist, connections, tools)
        }
    };

    // Active connection slugs (status filter mirrors the original logic).
    let connected_slugs: std::collections::HashSet<String> = connections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.normalized_toolkit())
        .filter(|toolkit| !toolkit.is_empty())
        .collect();

    // Deduplicate the allowlist so a backend that returns duplicates
    // doesn't produce dual entries downstream.
    let mut unique_toolkits: Vec<String> = allowlisted_toolkits.clone();
    unique_toolkits.sort();
    unique_toolkits.dedup();

    // Build one entry per allowlisted toolkit. Connected entries
    // carry their action catalogue; not-connected entries carry an
    // empty `tools` vec.
    let mut integrations: Vec<ConnectedIntegration> = Vec::with_capacity(unique_toolkits.len());
    for slug in &unique_toolkits {
        let connected = connected_slugs.contains(slug);
        // Anchor the prefix with an underscore so slugs that share
        // a text prefix (e.g. `git` vs `github`) don't false-match
        // each other's actions. `GMAIL_SEND_EMAIL` matches `gmail_`,
        // not just `gmail`, so siblings stay in their own buckets.
        let action_prefix = format!("{}_", slug.to_uppercase());
        let tools: Vec<ConnectedIntegrationTool> = if connected {
            // Apply the same curated-whitelist + user-scope filter the
            // meta-tool layer uses, so the integrations_agent prompt
            // only advertises actions the agent is actually allowed to
            // call. One pref load per toolkit (not per action).
            let pref = super::providers::load_user_scope_or_default(slug).await;
            let filtered: Vec<&super::types::ComposioToolSchema> = tools_by_toolkit
                .iter()
                .filter(|t| t.function.name.starts_with(&action_prefix))
                .filter(|t| super::providers::is_action_visible_with_pref(&t.function.name, &pref))
                .collect();
            tracing::debug!(
                toolkit = %slug,
                kept = filtered.len(),
                "[composio][scopes] integrations prompt action set"
            );
            filtered
                .into_iter()
                .map(|t| ConnectedIntegrationTool {
                    name: t.function.name.clone(),
                    description: t.function.description.clone().unwrap_or_default(),
                    parameters: t.function.parameters.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };

        integrations.push(ConnectedIntegration {
            toolkit: slug.clone(),
            description: toolkit_description(slug).to_string(),
            tools,
            connected,
        });
    }

    integrations.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));

    let connected_count = integrations.iter().filter(|i| i.connected).count();
    tracing::info!(
        total = integrations.len(),
        connected = connected_count,
        "[composio] fetch_connected_integrations: done"
    );
    for ci in &integrations {
        tracing::debug!(
            toolkit = %ci.toolkit,
            connected = ci.connected,
            tool_count = ci.tools.len(),
            "[composio] integration overview"
        );
    }

    Some(integrations)
}

/// Just-in-time fetch of every available action for a single Composio
/// toolkit, returned in the [`ConnectedIntegrationTool`] shape the
/// `integrations_agent` spawn path expects.
///
/// Unlike [`fetch_connected_integrations`] (which bulk-fetches every
/// connected toolkit's tools once per session and caches the result),
/// this helper is uncached and scoped to a single toolkit — meant to
/// be called at `integrations_agent` spawn time so the sub-agent's
/// prompt always reflects the toolkit's current action catalogue.
///
/// The filter `starts_with("{TOOLKIT}_")` matches
/// `fetch_connected_integrations_uncached`'s own namespacing rule so
/// siblings like `github` / `git` don't leak into each other's buckets.
///
/// Returns an empty vec when the backend has no actions for the
/// toolkit (valid steady state for a freshly-authorised integration
/// whose catalogue hasn't been published yet). Returns `Err` only for
/// transport / auth failures the caller should surface to the user.
pub async fn fetch_toolkit_actions(
    client: &ComposioClient,
    toolkit: &str,
) -> anyhow::Result<Vec<ConnectedIntegrationTool>> {
    let toolkit_slug = toolkit.trim();
    if toolkit_slug.is_empty() {
        anyhow::bail!("fetch_toolkit_actions: toolkit must not be empty");
    }
    tracing::debug!(toolkit = %toolkit_slug, "[composio] fetch_toolkit_actions");
    let resp = client
        .list_tools(Some(&[toolkit_slug.to_string()]))
        .await
        .map_err(|e| anyhow::anyhow!("list_tools failed for toolkit `{toolkit_slug}`: {e}"))?;
    let action_prefix = format!("{}_", toolkit_slug.to_uppercase());
    // Apply curated whitelist + user scope so spawn-time tool
    // discovery agrees with the bulk path and the meta-tool layer.
    let pref = super::providers::load_user_scope_or_default(toolkit_slug).await;
    let actions: Vec<ConnectedIntegrationTool> = resp
        .tools
        .into_iter()
        .filter(|t| t.function.name.starts_with(&action_prefix))
        .filter(|t| super::providers::is_action_visible_with_pref(&t.function.name, &pref))
        .map(|t| ConnectedIntegrationTool {
            name: t.function.name,
            description: t.function.description.unwrap_or_default(),
            parameters: t.function.parameters,
        })
        .collect();
    tracing::debug!(
        toolkit = %toolkit_slug,
        action_count = actions.len(),
        "[composio] fetch_toolkit_actions: done"
    );
    Ok(actions)
}

// ── Direct mode (BYO API key) ───────────────────────────────────────

/// Read the current Composio routing mode and whether a direct-mode API
/// key is stored. **The key itself is never returned** — only a boolean
/// flag so the UI can show a "Connected" / "Not set" status.
pub async fn composio_get_mode(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    let mode = config.composio.mode.trim().to_string();
    let key_present = crate::openhuman::credentials::get_composio_api_key(config)
        .map_err(|e| format!("[composio-direct] get_composio_api_key failed: {e}"))?
        .is_some();
    tracing::debug!(
        mode = %mode,
        key_present = key_present,
        "[composio-direct] get_mode"
    );
    let payload = serde_json::json!({
        "mode": mode,
        "api_key_set": key_present,
    });
    Ok(RpcOutcome::new(
        payload,
        vec![format!(
            "composio: mode={mode}, api_key={}",
            if key_present { "set" } else { "unset" }
        )],
    ))
}

/// Persist a user-provided Composio API key for direct mode and
/// (optionally) flip `config.composio.mode` over to `"direct"`.
///
/// **Logging redacts the key** — only its length and presence are
/// recorded. See the `[composio-direct]` debug-logging contract in
/// CLAUDE.md.
pub async fn composio_set_api_key(
    config: &Config,
    api_key: &str,
    activate_direct: bool,
) -> OpResult<RpcOutcome<serde_json::Value>> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio.set_api_key: api_key must not be empty".to_string());
    }
    tracing::debug!(
        key_len = trimmed.len(),
        activate_direct,
        "[composio-direct] set_api_key (redacted)"
    );

    crate::openhuman::credentials::store_composio_api_key(config, trimmed)
        .await
        .map_err(|e| format!("[composio-direct] store_composio_api_key failed: {e}"))?;

    let mode_log = if activate_direct {
        // Persist the mode flip too — we route through the standard
        // config save path so the snapshot, watchers, and reload paths
        // all observe it.
        let mut cfg_mut = crate::openhuman::config::rpc::load_config_with_timeout()
            .await
            .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
        cfg_mut.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.into();
        cfg_mut
            .save()
            .await
            .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;
        "mode=direct"
    } else {
        "mode unchanged"
    };

    let effective_mode: String = if activate_direct {
        "direct".to_string()
    } else {
        config.composio.mode.clone()
    };

    // [composio-cache] Broadcast a ComposioConfigChanged event so any
    // tenant-scoped caches (chat-runtime integrations snapshot, agent
    // tool catalogue, frontend useComposioIntegrations poll) can drop
    // stale entries and re-fetch against the new client. Without this
    // the chat panel keeps showing backend-tenant integrations even
    // though the user just switched to direct mode (#1710).
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConfigChanged {
            mode: effective_mode.clone(),
            api_key_set: true,
        },
    );
    tracing::debug!(
        mode = %effective_mode,
        "[composio-cache] published ComposioConfigChanged after set_api_key"
    );

    Ok(RpcOutcome::new(
        serde_json::json!({
            "stored": true,
            "mode": effective_mode,
        }),
        vec![format!("composio: api key stored ({mode_log})")],
    ))
}

/// Clear the stored direct-mode API key and reset
/// `config.composio.mode` back to `"backend"`.
pub async fn composio_clear_api_key(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    tracing::debug!("[composio-direct] clear_api_key");
    crate::openhuman::credentials::clear_composio_api_key(config)
        .await
        .map_err(|e| format!("[composio-direct] clear_composio_api_key failed: {e}"))?;

    let mut cfg_mut = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
    cfg_mut.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_BACKEND.into();
    cfg_mut
        .save()
        .await
        .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;

    // [composio-cache] Symmetric with composio_set_api_key — any
    // tenant-scoped caches that were populated while the user was in
    // direct mode must be invalidated when we drop back to backend
    // mode, otherwise the chat panel would keep showing the (now
    // empty) direct-tenant state instead of the live backend tenant.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConfigChanged {
            mode: "backend".to_string(),
            api_key_set: false,
        },
    );
    tracing::debug!("[composio-cache] published ComposioConfigChanged after clear_api_key");

    Ok(RpcOutcome::new(
        serde_json::json!({ "cleared": true, "mode": "backend" }),
        vec!["composio: api key cleared, mode reset to backend".into()],
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

// ── Helpers re-exported so callers can pull connection/tool types without
// reaching into the nested types module.
pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};
