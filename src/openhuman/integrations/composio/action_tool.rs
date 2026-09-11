//! Per-action Composio tool wrapper.
//!
//! A [`ComposioActionTool`] is a [`Tool`] that represents exactly one
//! Composio action (e.g. `GMAIL_SEND_EMAIL`). It holds the action's
//! name, description, and parameter JSON schema so the LLM's native
//! tool-calling path can validate arguments before they hit the wire.
//!
//! These are constructed **dynamically at spawn time** by the sub-agent
//! runner when `integrations_agent` is spawned with a `toolkit` argument —
//! one tool per action in the chosen toolkit. The generic
//! [`ComposioExecuteTool`](super::tools::ComposioExecuteTool) dispatcher
//! is deliberately excluded from `integrations_agent`'s tool list in that
//! path so the model doesn't see two ways to call the same action.
//!
//! Lifetime: these tools live for the duration of a single sub-agent
//! spawn. Rather than baking a `ComposioClient` at construction time
//! (which would silently bypass a mid-session
//! [`crate::openhuman::config::ComposioConfig::mode`] toggle — see
//! issue #1710), each tool keeps an [`Arc<Config>`] and resolves the
//! client per call through
//! [`create_composio_client`] so a user flip from
//! `mode = "backend"` to `mode = "direct"` is honoured on the next
//! tool invocation without restarting the session. Mirrors the agent-
//! tool migration in
//! [`super::tools::ComposioExecuteTool`].

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::providers::ToolScope;
use super::tools::resolve_action_scope;
use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

/// A single Composio action exposed as a first-class tool.
pub struct ComposioActionTool {
    /// Held instead of a pre-baked [`super::client::ComposioClient`] so
    /// the [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every invocation.
    ///
    /// Pre-fix this field was `client: ComposioClient`, which captured
    /// the backend-bound handle at sub-agent spawn time. Toggling
    /// `composio.mode = "direct"` mid-session invalidated other caches
    /// but left these per-action tools still routing through
    /// `staging-api.tinyhumans.ai/agent-integrations/composio/execute`
    /// — silently bypassing the direct-mode user's personal Composio
    /// tenant. Resolving the client per call via
    /// [`create_composio_client`] keeps dispatch in lockstep with the
    /// live config, matching
    /// [`super::tools::ComposioExecuteTool`]. See issue #1710.
    config: Arc<Config>,
    /// Action slug as-shipped to Composio, e.g. `"GMAIL_SEND_EMAIL"`.
    action_name: String,
    /// Human-readable description from the Composio tool-list response.
    description: String,
    /// Full JSON schema for the action's parameters. Falls back to
    /// `{"type":"object"}` when the upstream response omits it so the
    /// LLM still gets a valid (if loose) shape.
    parameters: Value,
    /// When set, all executions through this tool target a specific
    /// Composio connection. Used when the sub-agent is spawned for a
    /// particular account (e.g. "send from my work Gmail").
    connection_id: Option<String>,
    /// Per-turn contract gate (#4853). On the first call to this action the
    /// gate surfaces the action's FULL live input schema/description so the
    /// model composes well-formed arguments (e.g. correctly-quoted Gmail
    /// queries) instead of guessing from the thin spawn-time schema; the retry
    /// executes normally. Held per tool instance, which lives for one
    /// `integrations_agent` spawn, so "seen" is scoped to that turn.
    gate: super::contract_gate::ContractGate,
}

impl ComposioActionTool {
    pub fn new(
        config: Arc<Config>,
        action_name: String,
        description: String,
        parameters: Option<Value>,
    ) -> Self {
        Self::with_connection_id(config, action_name, description, parameters, None)
    }

    pub fn with_connection_id(
        config: Arc<Config>,
        action_name: String,
        description: String,
        parameters: Option<Value>,
        connection_id: Option<String>,
    ) -> Self {
        let parameters = parameters.unwrap_or_else(|| serde_json::json!({"type": "object"}));
        Self {
            config,
            action_name,
            description,
            parameters,
            connection_id,
            gate: super::contract_gate::ContractGate::new(),
        }
    }
}

/// Render a Composio action slug (`GMAIL_SEND_EMAIL`) as a sentence-cased
/// human phrase ("Gmail send email") for the chat processing timeline.
fn humanize_composio_action(slug: &str) -> String {
    let lower = slug.trim().to_ascii_lowercase().replace('_', " ");
    let lower = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => slug.to_string(),
    }
}

#[async_trait]
impl Tool for ComposioActionTool {
    fn name(&self) -> &str {
        &self.action_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        let mut schema = self.parameters.clone();
        if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
            props.entry("connection_id").or_insert_with(|| {
                serde_json::json!({
                    "type": "string",
                    "description": "Optional. Target a specific account when multiple are connected. Use the connection_id from Connected Integrations. Omit to use the default."
                })
            });
        }
        schema
    }

    fn permission_level(&self) -> PermissionLevel {
        // Conservative default: many actions mutate external state
        // (send mail, create issues, modify calendars). Match
        // ComposioExecuteTool's write-level treatment so channel
        // permission caps behave identically whether the model goes
        // through the dispatcher or a per-action tool.
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        super::tools::action_mutates_external_state(&self.action_name)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn display_label(&self, _args: &Value) -> Option<String> {
        // Composio slugs are UPPER_SNAKE (e.g. `GMAIL_SEND_EMAIL`). Render a
        // sentence-cased phrase ("Gmail send email") instead of the shouty
        // raw slug or a Title-Cased "GMAIL SEND EMAIL". The contextual target
        // (recipient/query) is filled by the trait-default `display_detail`.
        Some(humanize_composio_action(&self.action_name))
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Agent-level sandbox gate (issue #685, CodeRabbit follow-up on
        // PR #904) — mirrors the check in
        // [`super::tools::ComposioExecuteTool::execute`] so a read-only
        // agent cannot slip a mutating call through the per-action
        // surface. The dispatcher path (`composio_execute`) and this
        // per-action path are the only two routes to the Composio
        // backend; both must honour the same invariant. Today no
        // read-only agent spawns per-action tools (only
        // `integrations_agent` registers them and it is
        // `sandbox_mode = "none"`), so this is strict defense-in-depth
        // for any future configuration that pairs the two.
        if matches!(current_sandbox_mode(), Some(SandboxMode::ReadOnly)) {
            let scope = resolve_action_scope(&self.action_name).await;
            if matches!(scope, ToolScope::Write | ToolScope::Admin) {
                tracing::info!(
                    tool = %self.action_name,
                    scope = scope.as_str(),
                    "[composio][sandbox] per-action execute blocked: agent is read-only, action is {}",
                    scope.as_str()
                );
                return Ok(ToolResult::error(format!(
                    "{}: action is classified `{}` and is refused because the calling \
                     agent is in strict read-only mode. Only `read`-scoped actions are \
                     available to this agent.",
                    self.action_name,
                    scope.as_str()
                )));
            }
        }

        // [#1710 Wave 4 / #4853] Reload the live config snapshot ONCE, up front,
        // and use it for BOTH the contract-gate lookup and dispatch. A mid-session
        // `composio.mode` / credential / workspace change must route the gate's
        // live-catalog fetch and the actual execution through the SAME config;
        // consulting the captured spawn-time `self.config` here would let the gate
        // resolve (or skip) a contract against stale routing while dispatch used
        // fresh routing. Anchored to this tool's original config path rather than
        // re-resolving process-global `OPENHUMAN_WORKSPACE` (the tool is scoped to
        // the user/workspace it was created for).
        let live_config =
            match config_rpc::reload_config_snapshot_with_timeout(self.config.as_ref()).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        tool = %self.action_name,
                        error = %e,
                        "[composio] per-action execute: load_config failed"
                    );
                    return Ok(ToolResult::error(format!(
                        "{}: failed to load live config: {e}",
                        self.action_name
                    )));
                }
            };

        // Contract gate (#4853): the per-action tool is built from the thin
        // spawn-time `list_tools` schema (often `{"type":"object"}` with no
        // field descriptions), so the model guesses argument formats — most
        // visibly sending unquoted Gmail `query` strings that return zero
        // results. On the first call this turn, surface the action's FULL live
        // contract (input schema + description) as a recoverable tool error and
        // let the retry — now with the schema in context — execute. Degrades to
        // a normal execute whenever the contract can't be resolved (see
        // `contract_gate::consult`), so an unconfigured/offline client never
        // blocks the action. Uses `live_config` so gate routing matches dispatch.
        match super::contract_gate::consult(&self.gate, &live_config, &self.action_name, &args)
            .await
        {
            super::contract_gate::GateDecision::Surface(contract) => {
                tracing::info!(
                    tool = %self.action_name,
                    "[composio][contract-gate] returning full contract before first execute"
                );
                return Ok(ToolResult::error(contract));
            }
            super::contract_gate::GateDecision::Proceed => {}
        }

        // Inject `timeZone` / `singleEvents` defaults for Google
        // Calendar list slugs (issue #1714). The per-action surface is
        // the spawn-time tool an integrations sub-agent picks when it
        // wants a single Composio action, so the same defaults must
        // fire here as on the dispatcher path.
        let iana = super::googlecalendar_args::current_iana_timezone();
        tracing::debug!(
            target: "composio",
            slug = %self.action_name,
            iana = %iana,
            "[composio][per-action] applying calendar query defaults pre-dispatch"
        );
        let args = super::googlecalendar_args::apply_calendar_query_defaults(
            &self.action_name,
            Some(args),
            &iana,
        );

        let started = std::time::Instant::now();
        // Allow the agent to override the baked-in connection_id via args
        let runtime_connection_id = args
            .as_ref()
            .and_then(|v| v.get("connection_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let effective_connection_id = runtime_connection_id
            .as_deref()
            .or(self.connection_id.as_deref());
        // One call for both routes. The module owns the prepare/retry/error
        // -mapping pipeline that `execute_dispatch` used to hold here, so a
        // direct-mode toggle still takes effect immediately (#1710): the route
        // is reconciled from `live_config` on the way through, rather than
        // baked into a client this tool captured at construction.
        let res = super::module_client::call::<_, super::types::ComposioExecuteResponse>(
            &live_config,
            super::module_client::methods::EXECUTE,
            super::types::ComposioExecuteRequest {
                tool: self.action_name.clone(),
                arguments: args,
                connection_id: effective_connection_id.map(str::to_string),
            },
        )
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match res {
            Ok(resp) => {
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                // Mirror `ComposioExecuteTool::execute` (composio/tools.rs):
                // prefer the backend-rendered `markdownFormatted` for LLM
                // consumption when present, fall back to the raw JSON
                // envelope on absence or non-success. Keeps both routes
                // (dispatcher + per-action) consistent so the model sees
                // the same compact transcript regardless of which tool
                // surface integrations_agent picked.
                let body = if resp.successful {
                    match resp
                        .markdown_formatted
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(md) => md.to_string(),
                        None => serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                    }
                } else {
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into())
                };
                Ok(ToolResult::success(body))
            }
            Err(e) => {
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: false,
                        error: Some(e.clone()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(e))
            }
        }
    }
}

#[cfg(test)]
#[path = "action_tool_tests.rs"]
mod tests;
