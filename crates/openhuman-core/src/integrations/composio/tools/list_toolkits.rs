//! The `composio_list_toolkits` agent tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::live_config::live_composio_config;
use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::super::client::{create_composio_client, ComposioClientKind};

pub struct ComposioListToolkitsTool {
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (see [`ComposioExecuteTool`] doc for the
    /// bug this guards against — #1710).
    config: Arc<Config>,
}

impl ComposioListToolkitsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioListToolkitsTool {
    fn name(&self) -> &str {
        "composio_list_toolkits"
    }
    fn description(&self) -> &str {
        "List the Composio toolkits currently enabled on the backend allowlist. \
         Use this before calling composio_authorize or composio_list_tools to see what \
         is allowed (e.g. gmail, notion)."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        // Composio proxies to external SaaS (Gmail, Notion, …), so it
        // lives in the Workflow category and is picked up by sub-agents
        // with `category_filter = "skill"`.
        ToolCategory::Workflow
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[composio] tool list_toolkits.execute");
        // Mirror the mode-aware pattern in
        // `ops::composio_list_toolkits`. In direct mode there is no
        // server-side allowlist; the user's personal Composio account
        // governs availability, so we return an empty toolkits list
        // with an explanatory log instead of silently routing through
        // the backend tinyhumans tenant (#1710).
        let live_config =
            match live_composio_config(self.config.as_ref()).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "[composio] tool: load_config failed");
                    return Ok(ToolResult::error(format!(
                        "composio: failed to load live config: {e}"
                    )));
                }
            };
        let client = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_toolkits.execute: backend variant");
                client
            }
            Ok(ComposioClientKind::Direct(_)) => {
                tracing::info!(
                    "[composio-direct] list_toolkits.execute: direct mode active — \
                     returning empty toolkits list. Users manage available toolkits \
                     via app.composio.dev."
                );
                let resp = super::super::types::ComposioToolkitsResponse::default();
                return Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_list_toolkits failed: {e}"
                )));
            }
        };
        match client.list_toolkits().await {
            Ok(resp) => Ok(ToolResult::success(
                serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
            )),
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_toolkits failed: {e}"
            ))),
        }
    }
}

// ── composio_list_connections ───────────────────────────────────────
