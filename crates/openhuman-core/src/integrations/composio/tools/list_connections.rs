//! The `composio_list_connections` agent tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::live_config::live_composio_config;
use super::redact::redact_composio_outcome;
use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::super::client::{create_composio_client, direct_list_connections, ComposioClientKind};

pub struct ComposioListConnectionsTool {
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (#1710).
    config: Arc<Config>,
}

impl ComposioListConnectionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioListConnectionsTool {
    fn name(&self) -> &str {
        "composio_list_connections"
    }
    fn description(&self) -> &str {
        "List the user's **currently-connected** Composio integrations. \
         Only entries with status ACTIVE / CONNECTED are returned; pending, \
         revoked, or failed connections are filtered out. Use this to detect \
         newly-authorised integrations mid-session. Each entry has \
         {id, toolkit, status, createdAt}."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let outcome = Box::pin(self.execute_unredacted(args)).await;
        redact_composio_outcome(&self.config, outcome)
    }
}

impl ComposioListConnectionsTool {
    async fn execute_unredacted(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[composio] tool list_connections.execute");
        // Mirror `ops::composio_list_connections`: route through the mode-aware
        // factory so the agent sees the correct tenant's connections in both
        // backend and direct mode. Before this fix, direct mode returned an
        // empty list regardless of the user's actual Composio connections,
        // which caused the agent to incorrectly conclude that no integrations
        // were linked and prompt unnecessary re-authorization (#1710).
        let live_config = match live_composio_config(self.config.as_ref()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] list_connections.execute: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio_list_connections: failed to load live config: {e}"
                )));
            }
        };
        let mut resp = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_connections.execute: backend variant");
                client.list_connections().await.map_err(|e| {
                    anyhow::anyhow!("composio_list_connections (backend) failed: {e}")
                })?
            }
            Ok(ComposioClientKind::Direct(direct)) => {
                tracing::debug!("[composio-direct] list_connections.execute: direct variant");
                direct_list_connections(&direct).await.map_err(|e| {
                    // [#1166 / Sentry TAURI-RUST-X9] Symmetric error
                    // routing with `ops.rs::composio_list_connections`.
                    // The agent-tool path can also fire 401s when a
                    // direct-mode user has a bad API key — without this
                    // hook the failure escapes the classifier and lands
                    // as an unclassified Sentry event. Render WITH the
                    // `[composio-direct]` anchor BEFORE reporting so the
                    // classifier arm in `is_provider_user_state_message`
                    // (gated on that prefix) actually fires.
                    let rendered = format!(
                        "[composio-direct] composio_list_connections (direct) failed: {e:#}"
                    );
                    super::super::ops::report_composio_op_error("list_connections", &rendered);
                    anyhow::anyhow!("{rendered}")
                })?
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_list_connections failed: {e}"
                )));
            }
        };
        // Filter server-side-indistinguishable states — callers should only
        // see integrations the user can actually act on. Matches the same
        // ACTIVE/CONNECTED allowlist used by `fetch_connected_integrations_uncached`
        // so the tool output and the prompt's Delegation Guide agree on what
        // counts as "connected".
        resp.connections.retain(|c| c.is_active());
        tracing::debug!(
            count = resp.connections.len(),
            "[composio] list_connections.execute: returning active connections"
        );
        Ok(ToolResult::success(
            serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
        ))
    }
}

// ── composio_authorize ──────────────────────────────────────────────
