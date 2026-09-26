//! The `composio_list_tools` agent tool.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::live_config::live_composio_config;
use super::redact::redact_composio_outcome;
use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult};

use super::super::client::{create_composio_client, ComposioClientKind};
use super::super::types::ComposioToolsResponse;
use super::visibility::{
    empty_uncurated_toolkits_message, filter_list_tools_response, normalized_scope_toolkits,
    render_tools_markdown, retain_connected_tools,
};

pub struct ComposioListToolsTool {
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::config::ComposioConfig::mode`] toggle is
    /// honoured on every call. Resolving the client per call mirrors
    /// [`crate::integrations::composio::ops::composio_execute`] and avoids
    /// the staged-routing bug (#1710) where a long-lived backend client
    /// would survive a user switch into `direct` mode.
    config: Arc<Config>,
}

impl ComposioListToolsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioListToolsTool {
    fn name(&self) -> &str {
        "composio_list_tools"
    }
    fn description(&self) -> &str {
        "List Composio action tools available through the backend. By default only \
         actions for toolkits the user has actively connected are returned — pass \
         `include_unconnected=true` to see every allowlisted toolkit's actions \
         (useful when planning whether to call `composio_authorize` for a new toolkit). \
         Pass an optional `toolkits` array to further filter (e.g. [\"gmail\"]). The \
         result is a JSON object with a `tools` array of OpenAI function-calling \
         tool schemas; use the slug from each entry's `function.name` as the `tool` \
         argument when calling `composio_execute`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of toolkit slugs to filter by."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional Composio action tags to filter by \
                                    (OR semantics — multiple tags broaden the result, \
                                    e.g. [\"readOnlyHint\"] or [\"repos\", \"stars\"]). \
                                    Case-insensitive."
                },
                "include_unconnected": {
                    "type": "boolean",
                    "description": "When true, include actions from toolkits the user \
                                    has not connected yet. Defaults to false (only \
                                    connected toolkits)."
                }
            },
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let (config, outcome) = Box::pin(self.execute_unredacted(args, options)).await;
        redact_composio_outcome(&config, outcome)
    }

    fn supports_markdown(&self) -> bool {
        true
    }
}

impl ComposioListToolsTool {
    /// Returns the config actually used for dispatch alongside the outcome,
    /// so [`Tool::execute_with_options`] redacts against the same
    /// credential that ran — not the possibly-stale snapshot captured when
    /// this tool was registered.
    async fn execute_unredacted(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> (Config, anyhow::Result<ToolResult>) {
        let toolkits = args.get("toolkits").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        // tags is only forwarded to the backend when the request is explicitly
        // scoped to GitHub — it is the one toolkit where the backend honours the
        // param (other toolkits ignore it and passing it could cause unintended
        // filtering on future toolkit expansions).
        let raw_tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        let tags = if super::super::ops::should_forward_tags(toolkits.as_deref()) {
            raw_tags
        } else {
            None
        };
        let include_unconnected = args
            .get("include_unconnected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        tracing::debug!(
            ?toolkits,
            ?tags,
            include_unconnected,
            prefer_markdown = options.prefer_markdown,
            "[composio] tool list_tools.execute"
        );

        // Resolve the client through the mode-aware factory so a
        // direct-mode user does not silently get the backend
        // tinyhumans-tenant tool list. In direct mode we return an
        // empty `tools` array with an explanatory log, mirroring the
        // ops.rs `composio_list_toolkits` / `composio_list_connections`
        // pattern. Surfacing the empty list explicitly is correct
        // fail-mode: the alternative — falling through to the backend
        // path — is exactly the bug we're closing (#1710).
        let live_config = match live_composio_config(self.config.as_ref()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool: load_config failed");
                return (
                    self.config.as_ref().clone(),
                    Ok(ToolResult::error(format!(
                        "composio: failed to load live config: {e}"
                    ))),
                );
            }
        };
        let client = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_tools.execute: backend variant");
                client
            }
            Ok(ComposioClientKind::Direct(_)) => {
                tracing::info!(
                    "[composio-direct] list_tools.execute: direct mode active — \
                     returning empty tools list. Discovery is delegated to the user's \
                     personal Composio account; backend-tenant tools are intentionally \
                     NOT surfaced in direct mode."
                );
                let resp = ComposioToolsResponse::default();
                let mut result = ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                );
                if options.prefer_markdown {
                    result.markdown_formatted = Some(render_tools_markdown(&resp));
                }
                return (live_config, Ok(result));
            }
            Err(e) => {
                return (
                    live_config,
                    Ok(ToolResult::error(format!(
                        "composio_list_tools failed: {e}"
                    ))),
                );
            }
        };

        let outcome = match client
            .list_tools(toolkits.as_deref(), tags.as_deref())
            .await
        {
            Ok(mut resp) => {
                filter_list_tools_response(&live_config, &mut resp).await;
                let mut connected_toolkits: Option<HashSet<String>> = None;

                if !include_unconnected {
                    // Restrict to toolkits with an ACTIVE / CONNECTED
                    // account. Mirrors the same status allowlist used by
                    // composio_list_connections so this view and the
                    // prompt's Delegation Guide stay in sync.
                    match client.list_connections().await {
                        Ok(conns) => {
                            let connected: HashSet<String> = conns
                                .connections
                                .iter()
                                .filter(|c| c.is_active())
                                .map(|c| c.normalized_toolkit())
                                .filter(|t| !t.is_empty())
                                .collect();
                            let dropped = retain_connected_tools(&mut resp, &connected);
                            tracing::debug!(
                                connected_toolkits = connected.len(),
                                dropped,
                                kept = resp.tools.len(),
                                "[composio] list_tools restricted to connected toolkits"
                            );
                            connected_toolkits = Some(connected);
                        }
                        Err(e) => {
                            // Soft-fail: surface the issue to the agent
                            // so it can retry with include_unconnected
                            // rather than silently returning [].
                            return (
                                live_config,
                                Ok(ToolResult::error(format!(
                                    "composio_list_tools failed to fetch connections \
                                     (needed to filter to connected toolkits — pass \
                                     include_unconnected=true to skip this check): {e}"
                                ))),
                            );
                        }
                    }
                }

                if resp.tools.is_empty() {
                    let scoped_toolkits =
                        normalized_scope_toolkits(toolkits.as_deref(), connected_toolkits.as_ref());
                    if let Some(message) = empty_uncurated_toolkits_message(&scoped_toolkits) {
                        tracing::debug!(
                            toolkits = ?scoped_toolkits,
                            "[composio] list_tools empty for uncurated toolkit scope"
                        );
                        return (live_config, Ok(ToolResult::error(message)));
                    }
                }

                let mut result = ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                );
                if options.prefer_markdown {
                    result.markdown_formatted = Some(render_tools_markdown(&resp));
                }
                Ok(result)
            }
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_tools failed: {e}"
            ))),
        };
        (live_config, outcome)
    }
}
