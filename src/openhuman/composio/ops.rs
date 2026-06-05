//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.

/// Toolkits that honour the `tags` query param on the backend tool-list endpoint.
/// Expand this list when a new toolkit gains tag support.
const TAG_QUERYABLE_TOOLKITS: &[&str] = &["github"];

/// Returns `true` when `tags` should be forwarded to the backend.
///
/// Tags are forwarded when no toolkit filter is active (`None` / empty slice)
/// or when at least one requested toolkit is in [`TAG_QUERYABLE_TOOLKITS`].
/// This is `pub(crate)` so `tools.rs` can reuse it without duplicating the list.
pub(crate) fn should_forward_tags(toolkits: Option<&[String]>) -> bool {
    match toolkits {
        None => true,
        Some(kits) => {
            kits.is_empty()
                || kits.iter().any(|k| {
                    TAG_QUERYABLE_TOOLKITS
                        .iter()
                        .any(|t| k.trim().eq_ignore_ascii_case(t))
                })
        }
    }
}

use crate::openhuman::config::Config;
use crate::openhuman::memory::MemoryClient;
use crate::openhuman::memory_store::chunks::store as memory_tree_store;
use crate::openhuman::memory_store::chunks::types::SourceKind;
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
    build_composio_client, create_composio_client, direct_list_connections, direct_list_tools,
    ComposioClient, ComposioClientKind,
};
use super::providers::{
    agent_ready_toolkits, capability_matrix, get_provider, sync_state::SyncState, ProviderContext,
    ProviderUserProfile, SyncOutcome, SyncReason,
};
use super::types::{
    ComposioActiveTriggersResponse, ComposioAuthorizeResponse, ComposioAvailableTriggersResponse,
    ComposioCapabilitiesResponse, ComposioConnectionsResponse, ComposioCreateTriggerResponse,
    ComposioDeleteResponse, ComposioDisableTriggerResponse, ComposioEnableTriggerResponse,
    ComposioExecuteResponse, ComposioGithubReposResponse, ComposioToolkitsResponse,
    ComposioToolsResponse, ComposioTriggerHistoryResult,
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

/// True when the user has selected Composio **direct** mode but has not yet
/// configured an API key (neither in the keychain nor `config.toml`).
///
/// This is a valid, user-controlled *setup* state — the user just flipped to
/// direct mode and is about to paste their key — NOT an operation failure.
/// Callers short-circuit to an empty result instead of letting the
/// mode-aware factory bail with "composio direct mode selected but no api key
/// is configured", which the desktop UI's 5 s poll would otherwise funnel to
/// Sentry on every tick (TAURI-RUST-R4).
///
/// Key presence MUST mirror the factory's own resolution in
/// [`create_composio_client`] (`client.rs`): a key counts if it is in the
/// keychain (`credentials::get_composio_api_key`) **or** in `config.toml`
/// (`config.composio.api_key`). Checking only the keychain would wrongly
/// short-circuit to an empty list for a user who configured their key via
/// `config.toml`, hiding their real connections.
fn direct_mode_without_key(config: &Config) -> OpResult<bool> {
    if config.composio.mode.trim() != crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT {
        return Ok(false);
    }
    let has_key = crate::openhuman::credentials::get_composio_api_key(config)
        .map_err(|e| format!("[composio] get_composio_api_key failed: {e}"))?
        .or_else(|| {
            config
                .composio
                .api_key
                .as_ref()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
        })
        .is_some();
    Ok(!has_key)
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
pub(crate) fn report_composio_op_error<E: std::fmt::Display + ?Sized>(operation: &str, err: &E) {
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

pub async fn composio_list_capabilities(
    _config: &Config,
) -> OpResult<RpcOutcome<ComposioCapabilitiesResponse>> {
    tracing::debug!("[composio] rpc list_capabilities");
    let resp = ComposioCapabilitiesResponse {
        capabilities: capability_matrix(),
    };
    let count = resp.capabilities.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} capability row(s) listed")],
    ))
}

/// List every toolkit slug that ships an agent-ready curated catalog.
///
/// Connected toolkits that are NOT in this list can still be
/// authorized via OAuth, but the agent has no curated action surface
/// for them — the UI should label such connections as
/// "preview / agent integration coming soon" so users aren't led into
/// a broken `composio_list_tools` → max-iterations loop. See #2283.
pub async fn composio_list_agent_ready_toolkits(
) -> OpResult<RpcOutcome<super::types::ComposioAgentReadyToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_agent_ready_toolkits");
    let toolkits: Vec<String> = agent_ready_toolkits()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let count = toolkits.len();
    let resp = super::types::ComposioAgentReadyToolkitsResponse { toolkits };
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} agent-ready toolkit(s) listed")],
    ))
}

// ── Connections ─────────────────────────────────────────────────────

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    // [Sentry TAURI-RUST-R4] Direct mode with no API key yet is a valid,
    // user-controlled *setup* state — not an operation failure. The desktop
    // UI polls this RPC every 5 s; without this guard the mode-aware factory
    // bails ("composio direct mode selected but no api key is configured") on
    // every tick and the error funnels to Sentry until the user pastes a key
    // (~3.2 k events, single user, release 0.57.5). Mirror `periodic.rs`'s
    // graceful skip and return the truthful empty list (no key → no tenant →
    // no connections). The Settings → Composio panel drives the "enter your
    // key" prompt off the separate `api_key_set` status, so the user is still
    // told what to do. We return BEFORE `create_composio_client` and
    // `sync_cache_with_connections`, so no error is constructed and the
    // integrations cache is left untouched.
    if direct_mode_without_key(config)? {
        tracing::debug!(
            "[composio] list_connections: direct mode selected, no api key configured yet \
             — returning empty connection list (valid setup state, not an error)"
        );
        return Ok(RpcOutcome::new(
            ComposioConnectionsResponse {
                connections: Vec::new(),
            },
            vec!["composio: direct mode — no api key configured yet, 0 connection(s)".to_string()],
        ));
    }
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
            let resp = direct_list_connections(&direct).await.map_err(|e| {
                // [#1166 / Sentry TAURI-RUST-X9] Restore symmetric error
                // routing for the direct-mode branch. Without this hook the
                // direct-mode 401 ("Invalid API key …") wire shape bypassed
                // `report_error_or_expected` and leaked ~15.7k events in ~22h
                // — same UI 5 s poll + `periodic.rs` tick that the
                // backend branch (line ~266) was already classifying.
                //
                // Render WITH the `[composio-direct]` anchor BEFORE
                // reporting so the classifier arm in
                // `is_provider_user_state_message` (which gates on that
                // prefix) actually fires.
                let rendered = format!("[composio-direct] list_connections failed: {e:#}");
                report_composio_op_error("list_connections", &rendered);
                rendered
            })?;
            let active = resp.connections.iter().filter(|c| c.is_active()).count();
            let total = resp.connections.len();
            // Reconcile the integrations cache against this fresh live
            // snapshot from the user's own tenant — same defensive
            // behaviour as the backend path, so the chat runtime's
            // connected-toolkits view stays in sync within one poll
            // interval.
            sync_cache_with_connections(&resp.connections);
            let resp = enrich_connections_with_identity(resp);
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
    let resp = enrich_connections_with_identity(resp);
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
            super::oauth_handoff::authorize_with_meta_guard(&client, toolkit, extra_params)
                .await
                .map_err(|e| {
                    report_composio_op_error("authorize", &e);
                    let wrapped = super::oauth_handoff::wrap_authorize_rate_limit_error(toolkit, e);
                    format!("[composio] authorize failed: {wrapped:#}")
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
            super::oauth_handoff::direct_authorize_with_meta_guard(
                &direct,
                toolkit,
                &config.composio.entity_id,
            )
            .await
            .map_err(|e| {
                let wrapped = super::oauth_handoff::wrap_authorize_rate_limit_error(toolkit, e);
                // [#1166 / Sentry TAURI-RUST-X9] Symmetric with the
                // backend branch's `report_composio_op_error` on the
                // same handler — direct-mode 401s from
                // `connected_accounts/link` were leaking otherwise.
                // Render WITH the `[composio-direct]` anchor so the
                // classifier arm fires; wrapped error preserves any
                // rate-limit classifications fed up the ladder.
                let rendered = format!("[composio-direct] authorize failed: {wrapped:#}");
                report_composio_op_error("authorize", &rendered);
                rendered
            })?
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
    clear_memory: bool,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let toolkit = match resolve_toolkit_for_connection(&client, connection_id).await {
        Ok(toolkit) => Some(toolkit),
        Err(error) if clear_memory => {
            return Err(format!(
                "[composio] delete_connection cannot clear memory without resolving toolkit: {error}"
            ));
        }
        Err(_) => None,
    };
    let memory_targets = if clear_memory {
        composio_memory_targets_for_connection(config, toolkit.as_deref(), connection_id)
            .await
            .map_err(|error| {
                format!("[composio] delete_connection cannot enumerate memory targets: {error:#}")
            })?
    } else {
        Vec::new()
    };
    let mut resp = client.delete_connection(connection_id).await.map_err(|e| {
        report_composio_op_error("delete_connection", &e);
        format!("[composio] delete_connection failed: {e:#}")
    })?;
    let mut memory_chunks_deleted = 0;
    let mut memory_clear_errors = Vec::new();
    for target in &memory_targets {
        match target.delete(config) {
            Ok(deleted) => {
                memory_chunks_deleted += deleted;
            }
            Err(error) => {
                memory_clear_errors.push(format!(
                    "[composio] connection deleted, but failed to clear memory chunks for {}: {error:#}",
                    target.label()
                ));
            }
        }
    }
    resp.memory_chunks_deleted = memory_chunks_deleted;
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
    // Prune the local memory_sources registry entry for this connection.
    // The registry keys composio sources by `connection_id` and the
    // reconciler only ever upserts, so a deleted connection's
    // `[[memory_sources]]` entry is otherwise orphaned forever (and on
    // reconnect the backend mints a fresh `connection_id`, leaving the stale
    // one stranded). Best-effort: the backend connection is already gone, so
    // a config-save failure must not fail the whole delete — log and move on.
    match crate::openhuman::memory_sources::registry::remove_composio_source_by_connection_id(
        connection_id,
    )
    .await
    {
        Ok(0) => {}
        Ok(removed) => tracing::debug!(
            connection_id = %connection_id,
            removed,
            "[composio] pruned memory_sources entry after connection deletion"
        ),
        Err(e) => tracing::warn!(
            connection_id = %connection_id,
            error = %e,
            "[composio] failed to prune memory_sources entry after connection deletion (non-fatal)"
        ),
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
    if !memory_clear_errors.is_empty() {
        return Err(memory_clear_errors.join("; "));
    }
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoryCleanupTarget {
    Exact(SourceKind, String),
    Prefix(SourceKind, String),
    Owner(SourceKind, String),
}

impl MemoryCleanupTarget {
    fn delete(&self, config: &Config) -> anyhow::Result<usize> {
        match self {
            Self::Exact(source_kind, source_id) => {
                memory_tree_store::delete_chunks_by_source(config, *source_kind, source_id)
            }
            Self::Prefix(source_kind, source_id_prefix) => {
                memory_tree_store::delete_chunks_by_source_prefix(
                    config,
                    *source_kind,
                    source_id_prefix,
                )
            }
            Self::Owner(source_kind, owner) => {
                memory_tree_store::delete_chunks_by_owner(config, *source_kind, owner)
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Exact(source_kind, source_id) => {
                format!("{}:{source_id}", source_kind.as_str())
            }
            Self::Prefix(source_kind, source_id_prefix) => {
                format!("{}:{source_id_prefix}*", source_kind.as_str())
            }
            Self::Owner(source_kind, owner) => {
                format!("{}:owner:{owner}", source_kind.as_str())
            }
        }
    }
}

async fn composio_memory_targets_for_connection(
    config: &Config,
    toolkit: Option<&str>,
    connection_id: &str,
) -> anyhow::Result<Vec<MemoryCleanupTarget>> {
    let Some(toolkit) = toolkit.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };

    let targets = match toolkit.to_ascii_lowercase().as_str() {
        "slack" => vec![MemoryCleanupTarget::Exact(
            SourceKind::Chat,
            format!("slack:{connection_id}"),
        )],
        "gmail" => gmail_memory_sources_for_connection(connection_id),
        "notion" => notion_memory_targets_for_connection(config, connection_id).await?,
        "drive" | "googledrive" | "google_drive" => {
            drive_memory_targets_for_connection(connection_id)
        }
        _ => Vec::new(),
    };
    Ok(targets)
}

fn gmail_memory_sources_for_connection(connection_id: &str) -> Vec<MemoryCleanupTarget> {
    vec![
        MemoryCleanupTarget::Owner(SourceKind::Email, format!("gmail-sync:{connection_id}")),
        MemoryCleanupTarget::Exact(SourceKind::Email, format!("gmail:{connection_id}")),
        MemoryCleanupTarget::Prefix(SourceKind::Email, format!("gmail:{connection_id}:")),
        MemoryCleanupTarget::Prefix(SourceKind::Email, format!("gmail:{connection_id}/")),
    ]
}

async fn notion_memory_targets_for_connection(
    config: &Config,
    connection_id: &str,
) -> anyhow::Result<Vec<MemoryCleanupTarget>> {
    let mut targets = connection_scoped_document_targets("notion", connection_id);

    let memory = Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone()).map_err(|error| {
            anyhow::anyhow!(
                "failed to open memory client for notion cleanup target discovery: {error}"
            )
        })?,
    );
    let state = SyncState::load(&memory, "notion", connection_id)
        .await
        .map_err(|error| {
            anyhow::anyhow!("failed to load notion sync state for memory cleanup: {error}")
        })?;
    for raw_id in state.synced_ids {
        let Some(page_id) = notion_synced_page_id(&raw_id) else {
            continue;
        };
        targets.push(MemoryCleanupTarget::Exact(
            SourceKind::Document,
            format!("notion:{page_id}"),
        ));
        targets.push(MemoryCleanupTarget::Exact(
            SourceKind::Document,
            format!("composio-notion-page-{page_id}"),
        ));
    }

    Ok(dedupe_memory_targets(targets))
}

fn drive_memory_targets_for_connection(connection_id: &str) -> Vec<MemoryCleanupTarget> {
    ["drive", "googledrive", "google_drive"]
        .into_iter()
        .flat_map(|prefix| connection_scoped_document_targets(prefix, connection_id))
        .collect()
}

fn connection_scoped_document_targets(
    prefix: &str,
    connection_id: &str,
) -> Vec<MemoryCleanupTarget> {
    vec![
        MemoryCleanupTarget::Exact(SourceKind::Document, format!("{prefix}:{connection_id}")),
        MemoryCleanupTarget::Prefix(SourceKind::Document, format!("{prefix}:{connection_id}:")),
        MemoryCleanupTarget::Prefix(SourceKind::Document, format!("{prefix}:{connection_id}/")),
    ]
}

fn notion_synced_page_id(raw_id: &str) -> Option<String> {
    let page_id = raw_id.split_once('@').map_or(raw_id, |(id, _)| id).trim();
    (!page_id.is_empty()).then(|| page_id.to_string())
}

fn dedupe_memory_targets(targets: Vec<MemoryCleanupTarget>) -> Vec<MemoryCleanupTarget> {
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}

// ── Tools ───────────────────────────────────────────────────────────

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
    tags: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    let effective_tags = if should_forward_tags(toolkits.as_deref()) {
        tags
    } else {
        None
    };
    tracing::debug!(?toolkits, ?effective_tags, "[composio] rpc list_tools");
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
            let resp = client
                .list_tools(toolkits.as_deref(), effective_tags.as_deref())
                .await
                .map_err(|e| {
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
                        // [#1166 / Sentry TAURI-RUST-X9] Symmetric error
                        // routing — the prefetch call goes to the same v3
                        // `/connected_accounts` endpoint as `list_connections`
                        // and would emit the same 401 wire shape. Render
                        // WITH the `[composio-direct]` anchor so the
                        // classifier arm fires on the prefetch path too.
                        let rendered = format!(
                            "[composio-direct] list_tools: prefetch connections failed: {e:#}"
                        );
                        report_composio_op_error("list_connections", &rendered);
                        rendered
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
                ?effective_tags,
                "[composio-direct] list_tools: fetching v3 tool schemas"
            );
            // Forward the same `effective_tags` the backend branch uses so the
            // tag filter is honoured in direct (BYO-key) mode too — previously
            // it was computed above and then dropped on this branch.
            let mut resp = direct_list_tools(&direct, &scope, effective_tags.as_deref())
                .await
                .map_err(|e| {
                    // [#1166 / Sentry TAURI-RUST-X9] Symmetric with the backend
                    // branch's hook (line ~451). Direct-mode `list_tools`
                    // failures are user-state when the API key is bad. Render
                    // WITH the `[composio-direct]` anchor so the classifier
                    // arm fires.
                    let rendered = format!("[composio-direct] list_tools failed: {e:#}");
                    report_composio_op_error("list_tools", &rendered);
                    rendered
                })?;
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
    // Centralized prepare → retry → error-mapping pipeline (#1797),
    // mode-aware over the backend/direct split (#1710). The dispatcher
    // returns pre-formatted `[composio:error:<class>] …` strings so the
    // frontend formatter at `app/src/lib/composio/formatters.ts` can
    // parse the class regardless of which mode produced the failure.
    let result = super::execute_dispatch::execute_composio_action_kind(
        kind,
        tool,
        arguments,
        &config.composio.entity_id,
    )
    .await;
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
            // Preserve already-classified errors from the dispatcher
            // (`[composio:error:<class>] …`) so the frontend formatter at
            // `app/src/lib/composio/formatters.ts` can still parse the class.
            let is_classified = e.starts_with("[composio:error:");
            tracing::debug!(
                tool = %tool,
                elapsed_ms,
                classified = is_classified,
                "[composio] rpc execute error mapped"
            );
            if is_classified {
                Err(e)
            } else {
                Err(format!("[composio] execute failed: {e}"))
            }
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
            // Keep the raw error on the Sentry funnel for diagnosis (unchanged).
            report_composio_op_error("enable_trigger", &e);
            // Map the backend error (e.g. a 403 "you do not have permission to
            // enable triggers on this connection") into actionable, user-facing
            // guidance instead of leaking the raw blob to the UI (issue #2913).
            let raw = format!("{e:#}");
            let class = super::error_mapping::classify_composio_error(slug, &raw);
            let mapped = super::error_mapping::format_provider_error(slug, &raw);
            tracing::warn!(
                slug = %slug,
                connection_id = %connection_id,
                class = class.as_str(),
                "[composio] enable_trigger failed; surfacing mapped error"
            );
            mapped
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
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
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
            usage: Default::default(),
            max_items: None,
            sync_depth_days: None,
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
        "[composio] rpc sync (spawned)"
    );
    // Validate synchronously — a bad request (unknown connection / no native
    // provider for toolkit) must surface to the caller via the RPC error
    // envelope, not silently inside a spawned task.
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;
    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;
    let _ = client; // see analogous comment above — drop the pre-baked client (#1710).

    // `provider.sync` walks every page of the upstream API and ingests every
    // message in-band — on a real prod inbox a healthy run can legitimately
    // exceed the frontend's 30s `composio_sync` RPC `.await` cap (one
    // healthy periodic tick is already ~100s for 20 pages / 500 messages).
    // There is no reason for the UI to block on it: per-source progress is
    // already exposed via the polled `openhuman.memory_sync_status_list` RPC,
    // which reads `mem_tree_chunks` directly and therefore reflects the
    // spawned task's per-message ingest in real time. So we spawn the sync
    // as a background task and return immediately with a "started" envelope.
    // The periodic scheduler (`composio::periodic`) already runs
    // `provider.sync` from inside its own `tokio::spawn` loop — same pattern.
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let toolkit_for_outcome = toolkit.clone();
    let connection_id_for_log = connection_id.to_string();

    tokio::spawn(async move {
        let toolkit_in_task = ctx.toolkit.clone();
        match provider.sync(&ctx, reason).await {
            Ok(out) => {
                tracing::info!(
                    toolkit = %toolkit_in_task,
                    connection_id = %connection_id_for_log,
                    items_ingested = out.items_ingested,
                    elapsed_ms = out.elapsed_ms(),
                    "[composio] background sync ok"
                );
            }
            Err(e) => {
                report_composio_op_error("sync", &e);
                tracing::warn!(
                    toolkit = %toolkit_in_task,
                    connection_id = %connection_id_for_log,
                    error = %e,
                    "[composio] background sync failed"
                );
            }
        }
    });

    let summary = format!("composio: {toolkit_for_outcome} sync started (background)");
    let outcome = SyncOutcome {
        toolkit: toolkit_for_outcome,
        connection_id: Some(connection_id.to_string()),
        reason: reason.as_str().to_string(),
        items_ingested: 0,
        started_at_ms,
        // Sentinel: still running. Frontend should rely on
        // `memory_sync_status_list` for progress; `finished_at_ms == 0`
        // means "spawned, not yet complete".
        finished_at_ms: 0,
        summary: summary.clone(),
        details: serde_json::json!({ "status": "started" }),
    };
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

/// Enrich each [`super::types::ComposioConnection`] with human-readable
/// identity fields (`account_email`, `workspace`, `username`) from the
/// persisted provider profile cache so the UI picker can show
/// "Gmail · user@example.com" instead of a generic "Account N" label.
///
/// This is best-effort — no live API calls are made (one SQLite read per poll).
/// If the memory client is not ready yet (first launch before any sync)
/// or no profile rows exist for a connection, that connection is returned
/// unchanged and the UI falls back to its numbered label logic.
///
/// Connections that already carry identity fields (e.g. from the
/// backend-proxied path) are left untouched.
fn enrich_connections_with_identity(
    mut resp: super::types::ComposioConnectionsResponse,
) -> super::types::ComposioConnectionsResponse {
    use std::collections::HashMap;

    use super::providers::profile::{load_connected_identities, normalize_connection_identifier};

    let identities = load_connected_identities();
    if identities.is_empty() {
        tracing::debug!(
            "[composio] enrich_connections_with_identity: no cached identities yet \
             — picker will fall back to numbered labels until first sync completes"
        );
        return resp;
    }

    // (normalized_toolkit, normalized_conn_id) → identity
    let lookup: HashMap<(String, String), _> = identities
        .iter()
        .map(|id| {
            (
                (
                    normalize_connection_identifier(&id.source),
                    normalize_connection_identifier(&id.identifier),
                ),
                id,
            )
        })
        .collect();

    tracing::debug!(
        total = resp.connections.len(),
        cached_identities = identities.len(),
        "[composio] enrich_connections_with_identity: enriching connection labels"
    );

    for conn in &mut resp.connections {
        // Skip connections already carrying identity info (backend-proxied
        // path may supply them directly).
        if conn.account_email.is_some() || conn.workspace.is_some() || conn.username.is_some() {
            continue;
        }
        let toolkit_key = normalize_connection_identifier(&conn.toolkit);
        let conn_id_key = normalize_connection_identifier(&conn.id);
        if let Some(identity) = lookup.get(&(toolkit_key, conn_id_key)) {
            conn.account_email = identity.email.clone();
            // display_name carries the user's name; for Slack it falls back
            // to the team/workspace name when no per-user display name exists,
            // making it the best available workspace signal.
            conn.workspace = identity.display_name.clone();
            conn.username = identity.handle.clone();
            tracing::debug!(
                toolkit = %conn.toolkit,
                connection_id = %conn.id,
                has_email = conn.account_email.is_some(),
                has_workspace = conn.workspace.is_some(),
                has_username = conn.username.is_some(),
                "[composio] enrich_connections_with_identity: enriched connection"
            );
        }
    }
    resp
}

#[cfg(test)]
pub(crate) use super::connected_integrations::cache_key;
use super::connected_integrations::sync_cache_with_connections;
pub use super::connected_integrations::{
    cached_active_integrations, connected_set_hash, fetch_connected_integrations,
    fetch_connected_integrations_status, fetch_toolkit_actions,
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};
#[cfg(test)]
pub(crate) use super::connected_integrations::{CachedIntegrations, CACHE_TTL, INTEGRATIONS_CACHE};
#[cfg(test)]
pub(crate) use crate::openhuman::context::prompt::ConnectedIntegration;
#[cfg(test)]
pub(crate) use std::time::{Duration, Instant};

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
