//! The `composio_connect` agent tool — an inline chat approval card that
//! connects a Composio toolkit without sending the user to Connections
//! (issue #3993), plus its toolkit-slug canonicalisation, park-timeout
//! resolution, and fresh liveness check.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::live_config::live_composio_config;
use super::redact::redact_composio_outcome;
use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::super::client::{create_composio_client, direct_list_connections, ComposioClientKind};

// ── composio_connect (inline approval card, #3993) ──────────────────

/// Canonicalize an agent/user-supplied toolkit slug to the form Composio's
/// backend expects. Mirrors `canonicalizeComposioToolkitSlug` on the FE
/// (`app/src/lib/composio/toolkitSlug.ts`) — **keep the alias maps in sync**.
/// The agent frequently guesses `google_drive` where Composio uses
/// `googledrive` (#3993); without this the OAuth handoff fails with an opaque
/// error.
pub(super) fn canonicalize_toolkit_slug(slug: &str) -> String {
    let key = slug.trim().to_ascii_lowercase();
    match key.as_str() {
        "feishu" | "lark" => "larksuite".to_string(),
        "google_calendar" => "googlecalendar".to_string(),
        "google_drive" => "googledrive".to_string(),
        "google_sheets" => "googlesheets".to_string(),
        _ => key,
    }
}

/// Default bound (seconds) for how long [`ComposioConnectTool`] parks on the
/// inline-connect approval card before giving up (issue #4756).
///
/// The gate's own TTL is up to ten minutes (`DEFAULT_APPROVAL_TTL` in
/// `approval::gate`). That is fine when a human is watching the card, but when
/// the card can't be resolved — a headless/eval run, or a chat turn whose
/// client has since disconnected — `composio_connect` would otherwise block the
/// whole turn for minutes and deliver an empty reply, while the read path
/// (`composio_list_connections`) returns a graceful "not connected" prompt in
/// seconds. Bounding the park keeps the interactive resume-in-turn UX for a
/// present user (a click + OAuth round-trip completes well inside it) while
/// guaranteeing the act path degrades to a fast connect prompt instead of
/// hanging. Generous by design; env-overridable, `0` restores the full gate TTL.
pub(super) const DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS: u64 = 120;

/// Resolve the connect-card park bound. Reads
/// `OPENHUMAN_COMPOSIO_CONNECT_TIMEOUT_SECS`; `0` means "no composio-side bound"
/// (`None`) → fall back to the gate's own TTL.
pub(super) fn composio_connect_timeout() -> Option<std::time::Duration> {
    parse_composio_connect_timeout(
        std::env::var("OPENHUMAN_COMPOSIO_CONNECT_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`composio_connect_timeout`], kept env-free so it is
/// deterministically unit-testable. An absent/unparseable value falls back to
/// [`DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS`]; `0` yields `None` (opt out of the
/// composio-side bound).
pub(super) fn parse_composio_connect_timeout(
    env_value: Option<&str>,
) -> Option<std::time::Duration> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS);
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// Fresh (uncached) liveness check for `toolkit`.
///
/// Tri-state via `Result`:
/// - `Ok(true)`  — a connection is verified ACTIVE.
/// - `Ok(false)` — the read succeeded but no ACTIVE connection exists.
/// - `Err(_)`    — the state could **not** be verified (client construction or
///   the list call failed).
///
/// Used to confirm liveness after the approval gate resolves `Allow`. The
/// card-driven path only approves once it has polled the connection ACTIVE,
/// but other approval surfaces (a typed `yes`, Telegram's approval prompt, or
/// an existing auto-approve entry) resolve `Allow` with no OAuth poll — so
/// `Allow` alone must NOT be treated as "connected" (#3993, codex review).
///
/// Distinguishing `Err` from `Ok(false)` lets the caller fail closed on a
/// transient backend/auth failure **without** fabricating an "OAuth not
/// complete" reason that wrongly blames the user (#4062, coderabbit review).
pub(super) async fn connection_is_active(config: &Config, toolkit: &str) -> anyhow::Result<bool> {
    let active_match = |connections: &[super::super::types::ComposioConnection]| {
        connections
            .iter()
            .any(|c| c.is_active() && c.normalized_toolkit().eq_ignore_ascii_case(toolkit))
    };
    match create_composio_client(config)? {
        ComposioClientKind::Backend(client) => {
            Ok(active_match(&client.list_connections().await?.connections))
        }
        ComposioClientKind::Direct(direct) => Ok(active_match(
            &direct_list_connections(&direct).await?.connections,
        )),
    }
}

/// Connect a Composio integration **inline in the chat** instead of
/// sending the user off to Connections.
///
/// Unlike [`ComposioAuthorizeTool`] (which hands the agent a raw
/// `connectUrl` it is not allowed to paste), this tool raises an
/// approval card via the process-global `ApprovalGate`. The frontend
/// recognises `tool_name == "composio_connect"` and renders a **Connect**
/// button: clicking it runs the existing `composio_authorize` RPC, opens
/// the OAuth handoff, and polls `composio_list_connections` until the
/// toolkit is ACTIVE — at which point it resolves the gate. The agent
/// then resumes the original task in the same turn.
pub struct ComposioConnectTool {
    config: Arc<Config>,
}

impl ComposioConnectTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioConnectTool {
    fn name(&self) -> &str {
        "composio_connect"
    }
    fn description(&self) -> &str {
        "Connect a Composio integration (OAuth) for the user **inline in the chat**. \
         Raises an approval card with a Connect button — the user authorizes in one \
         click without leaving the conversation, and this tool returns once the \
         connection is active (or the user declines). ALWAYS prefer this over telling \
         the user to open Connections. Returns {toolkit, connected}."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkit": {
                    "type": "string",
                    "description": "Toolkit slug to connect, e.g. 'gmail' or 'notion'."
                }
            },
            "required": ["toolkit"],
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }
    // NOTE: `external_effect` deliberately stays `false`. Gating happens
    // *inside* `execute` via a manual `ApprovalGate` intercept so we can
    // (a) skip the card when the toolkit is already connected and (b) carry
    // the toolkit slug into the card for the inline Connect button. The
    // engine's auto-gate is unconditional and would double-prompt.
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let outcome = Box::pin(self.execute_unredacted(args)).await;
        redact_composio_outcome(&self.config, outcome)
    }
}

impl ComposioConnectTool {
    async fn execute_unredacted(&self, args: Value) -> anyhow::Result<ToolResult> {
        let raw_toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if raw_toolkit.is_empty() {
            return Ok(ToolResult::error("composio_connect: 'toolkit' is required"));
        }
        // Canonicalize before any backend call — the agent often guesses
        // `google_drive` where Composio expects `googledrive` (#3993).
        let toolkit = canonicalize_toolkit_slug(raw_toolkit);
        tracing::debug!(raw = %raw_toolkit, toolkit = %toolkit, "[composio] tool connect.execute");

        // The inline connect card only has a surface on an interactive chat
        // turn (the web-chat path installs `APPROVAL_CHAT_CONTEXT`). On
        // background / cron turns there is no UI to click Connect, so fail
        // closed with a clear message rather than parking forever — mirrors
        // the `install_tool` guard (#3993).
        if crate::security::approval::APPROVAL_CHAT_CONTEXT
            .try_with(|_| ())
            .is_err()
        {
            return Ok(ToolResult::error(format!(
                "[policy-denied] composio_connect needs an interactive chat turn. \
                 Ask the user to connect '{toolkit}' in Connections."
            )));
        }

        // Skip the card when the toolkit is already connected.
        let live_config = match live_composio_config(self.config.as_ref()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] connect.execute: load_config failed");
                self.config.as_ref().clone()
            }
        };
        let already_connected = super::super::fetch_connected_integrations(&live_config)
            .await
            .into_iter()
            .any(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(&toolkit));
        if already_connected {
            tracing::debug!(toolkit = %toolkit, "[composio] connect.execute: already connected");
            return Ok(ToolResult::success(serde_json::to_string(&json!({
                "toolkit": toolkit,
                "connected": true,
                "already_connected": true,
            }))?));
        }

        // Validate the toolkit against the *connectable* catalog before raising
        // a card. The orchestrator only knows which toolkits are already
        // connected — it must NOT confabulate "unsupported" from that list
        // (#3993). This grounds the answer: a backend-allowlisted toolkit gets
        // a card; a genuinely unsupported one gets a clear, listed refusal
        // instead of a card that would fail on Connect.
        match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                if let Ok(resp) = client.list_toolkits().await {
                    // Empty allowlist = backend predates the catalog / unknown;
                    // don't block — let the OAuth handoff report support.
                    if !resp.toolkits.is_empty()
                        && !resp
                            .toolkits
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(&toolkit))
                    {
                        let available = resp.toolkits.join(", ");
                        tracing::info!(toolkit = %toolkit, "[composio] connect.execute: toolkit not in allowlist");
                        return Ok(ToolResult::error(format!(
                            "composio_connect: '{toolkit}' is not an available integration. \
                             Connectable toolkits: {available}"
                        )));
                    }
                }
            }
            Ok(ComposioClientKind::Direct(_)) => {
                // Personal-tenant (direct) mode performs OAuth at app.composio.dev,
                // not via the backend handoff the card drives — so an inline card
                // can't complete it. Point the user to Settings instead.
                return Ok(ToolResult::error(format!(
                    "composio_connect: direct Composio mode is active — connect '{toolkit}' \
                     in Connections (your personal Composio account)."
                )));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_connect: Composio is unavailable: {e}"
                )));
            }
        }

        // Raise the inline connect card via the approval gate. The frontend
        // resolves it with `approve_once` once it polls the connection ACTIVE;
        // an explicit decline or the 10-minute TTL resolves it as denied.
        let gate = match crate::security::approval::ApprovalGate::try_global() {
            Some(g) => g,
            None => {
                return Ok(ToolResult::error(
                    "composio_connect: approval gate unavailable in this environment",
                ));
            }
        };
        let summary = format!("Connect {toolkit} to complete your task");
        // Bound the park (issue #4756). The gate parks up to its full TTL (10
        // min) waiting for the inline connect card to resolve; when nothing
        // resolves it — a headless/eval run, or a chat client that has since
        // disconnected — that otherwise blocks the whole turn to an empty reply.
        // `intercept_audited_bounded` caps the park at `composio_connect_timeout()`
        // and, when that bound elapses, abandons the park *inside the gate* in a
        // cancellation-safe way (waiter evicted + thread/meeting routing cleared,
        // but the `pending_approvals` row left open so a later human card-click
        // still resolves it in the DB and a re-ask sees it already-connected). It
        // returns `None` on that elapse, so we degrade to a fast, actionable
        // connect prompt — matching the read path — rather than hanging. We bound
        // through the gate (not an outer `tokio::time::timeout` that would drop
        // the parked future and orphan the waiter/routing) per the codex review
        // on this PR. The reply is shaped so the agent RELAYS it and does NOT
        // immediately retry `composio_connect` (a retry would just park again).
        let (outcome, _request_id) = match gate
            .intercept_audited_bounded(
                "composio_connect",
                &summary,
                json!({ "toolkit": toolkit }),
                composio_connect_timeout(),
            )
            .await
        {
            Some(resolved) => resolved,
            None => {
                tracing::info!(
                    toolkit = %toolkit,
                    "[composio] connect.execute: approval card not resolved within bound — \
                     returning a fast connect prompt instead of parking the turn (#4756)"
                );
                return Ok(ToolResult::success(serde_json::to_string(&json!({
                    "toolkit": toolkit,
                    "connected": false,
                    "pending": true,
                    "reason": format!(
                        "A Connect card for {toolkit} was raised but wasn't completed in time \
                         (no one authorized it). Tell the user to click Connect on the card, or \
                         connect {toolkit} in Connections, then ask again once it's \
                         done. Do not call composio_connect again until they confirm."
                    ),
                }))?));
            }
        };
        match outcome {
            crate::security::approval::GateOutcome::Allow => {
                // `Allow` only means the prompt was approved — re-check liveness
                // with a fresh read, because non-card approval surfaces (typed
                // "yes", Telegram, auto-approve) resolve Allow without running
                // the OAuth poll (#3993, codex review).
                match connection_is_active(&live_config, &toolkit).await {
                    Ok(true) => {
                        tracing::debug!(toolkit = %toolkit, "[composio] connect.execute: connection active");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": true,
                        }))?))
                    }
                    Ok(false) => {
                        tracing::info!(toolkit = %toolkit, "[composio] connect.execute: approved but not yet active");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": false,
                            "reason": "Approved, but the connection is not active yet — the user still needs to complete the OAuth flow.",
                        }))?))
                    }
                    Err(e) => {
                        // Couldn't verify liveness — a transient backend/auth
                        // failure, NOT proof the user skipped OAuth. Fail closed
                        // (connected:false) but report a verification error
                        // rather than fabricating an "OAuth incomplete" reason
                        // that blames the user and can drive the agent into
                        // reconnect loops (#4062, coderabbit review).
                        tracing::warn!(toolkit = %toolkit, error = %e, "[composio] connect.execute: liveness check failed");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": false,
                            "reason": "Approved, but the connection state could not be verified right now (a temporary problem reaching Composio). Please try connecting again in a moment.",
                        }))?))
                    }
                }
            }
            crate::security::approval::GateOutcome::Deny { reason } => {
                tracing::info!(toolkit = %toolkit, reason = %reason, "[composio] connect.execute: declined");
                Ok(ToolResult::success(serde_json::to_string(&json!({
                    "toolkit": toolkit,
                    "connected": false,
                    "declined": true,
                    "reason": reason,
                }))?))
            }
        }
    }
}
