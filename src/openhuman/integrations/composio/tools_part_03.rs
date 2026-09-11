
#[async_trait]
impl Tool for ComposioExecuteTool {
    fn name(&self) -> &str {
        "composio_execute"
    }
    fn description(&self) -> &str {
        "Execute a Composio action by slug. `tool` is the action slug returned from \
         composio_list_tools (e.g. 'GMAIL_SEND_EMAIL'); `arguments` is an object that \
         conforms to that tool's parameter schema. Returns the provider result plus \
         cost (USD)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "Composio action slug, e.g. 'GMAIL_SEND_EMAIL'."
                },
                "arguments": {
                    "type": "object",
                    "description": "Action-specific arguments. Shape depends on the tool."
                },
                "connection_id": {
                    "type": "string",
                    "description": "Optional. Target a specific account when multiple are connected for a toolkit. Use the connection_id from '## Connected Integrations'. Omit to use the default account."
                }
            },
            "required": ["tool"],
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        // Some composio actions send emails, create files, etc. — treat
        // as write-level to respect channel permission caps.
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    fn external_effect_with_args(&self, args: &Value) -> bool {
        args.get("tool")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
            .map(action_mutates_external_state)
            .unwrap_or(true)
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let tool = args
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if tool.is_empty() {
            return Ok(ToolResult::error(
                "composio_execute: 'tool' is required (e.g. GMAIL_SEND_EMAIL)",
            ));
        }
        let arguments = args.get("arguments").cloned();
        let connection_id = args
            .get("connection_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // INFO-level entry log — visible without bumping log level. Logs
        // ARG KEYS only by default so traces don't leak email bodies / PII;
        // the full arg JSON goes through a paired DEBUG log below for deep
        // debugging. Together these let `grep "[composio][execute]"` show
        // the full agent→backend audit trail at default verbosity.
        let arg_keys: Vec<&str> = arguments
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();
        tracing::info!(
            tool = %tool,
            connection_id = %connection_id,
            arg_keys = ?arg_keys,
            "[composio][execute] >> dispatch"
        );
        if let Some(ref args_json) = arguments {
            tracing::debug!(
                tool = %tool,
                args = %args_json,
                "[composio][execute] >> full args (DEBUG)"
            );
        }

        // Agent-level sandbox gate (issue #685) — applies on top of the
        // user's scope preference below. When the currently-executing
        // agent declares `sandbox_mode = "read_only"` in its
        // `agent.toml`, we refuse to dispatch any Write- or Admin-scoped
        // composio action regardless of what the user's scope pref
        // allows, so a strictly-read-only agent (planner, critic,
        // morning_briefing, …) can never mutate user state via the
        // composio surface. `SandboxMode::None` / `Sandboxed` (and the
        // `None` task-local value used by direct CLI / JSON-RPC / unit
        // tests) pass through unchanged.
        if matches!(current_sandbox_mode(), Some(SandboxMode::ReadOnly)) {
            let scope = resolve_action_scope(&tool).await;
            if matches!(scope, ToolScope::Write | ToolScope::Admin) {
                tracing::info!(
                    tool = %tool,
                    scope = scope.as_str(),
                    "[composio][sandbox] execute blocked: agent is read-only, action is {}",
                    scope.as_str()
                );
                return Ok(ToolResult::error(format!(
                    "composio_execute: action `{tool}` is classified `{}` and is refused \
                     because the calling agent is in strict read-only mode. Only `read`-scoped \
                     actions are available to this agent.",
                    scope.as_str()
                )));
            }
        }

        // Enforce per-user scope preferences before delegating to backend.
        // Uses `self.config` rather than the live-reloaded config resolved
        // further down: a scope-pref read is not the direct-mode-toggle
        // hazard the composio client resolution below guards against, and
        // restructuring the reload earlier would move it ahead of the
        // sandbox/task-window setup this function does first.
        match evaluate_tool_visibility(self.config.as_ref(), &tool).await {
            ToolDecision::Allow | ToolDecision::PassthroughCheckScope { .. } => {}
            ToolDecision::BlockedByScope { scope } => {
                let toolkit = toolkit_from_slug(&tool).unwrap_or_default();
                let pref = load_user_scope_pref(self.config.as_ref(), &toolkit).await;
                let msg = scope_error_message(&tool, scope, pref);
                tracing::info!(
                    tool = %tool,
                    toolkit = %toolkit,
                    scope = scope.as_str(),
                    "[composio][scopes] execute blocked by user scope pref"
                );
                return Ok(ToolResult::error(msg));
            }
            ToolDecision::NotCurated => {
                let toolkit = toolkit_from_slug(&tool).unwrap_or_default();
                tracing::info!(
                    tool = %tool,
                    toolkit = %toolkit,
                    "[composio][scopes] execute blocked: action not in curated whitelist"
                );
                return Ok(ToolResult::error(format!(
                    "composio_execute: action `{tool}` is not in the curated whitelist for \
                     toolkit `{toolkit}`. Use composio_list_tools to see available actions."
                )));
            }
        }

        // Inject `timeZone` / `singleEvents` defaults for Google
        // Calendar list slugs so the host's IANA zone reaches the API
        // regardless of how the model built the args (issue #1714).
        // No-op for every other slug; respects caller-supplied values.
        let iana = super::googlecalendar_args::current_iana_timezone();
        tracing::debug!(
            target: "composio",
            slug = %tool,
            iana = %iana,
            "[composio][dispatcher] applying calendar query defaults pre-dispatch"
        );
        let arguments =
            super::googlecalendar_args::apply_calendar_query_defaults(&tool, arguments, &iana);

        // Task-recency window (morning briefing): when the calling agent
        // installed a window, inject best-effort server-side narrowing for
        // curated task-fetch slugs. No-op for every other slug and for
        // normal chat / CLI / JSON-RPC (window unset → None). The
        // authoritative enforcement is the post-filter on the response below.
        let task_window_since = current_task_recency_window().map(|w| {
            chrono::Utc::now()
                - chrono::Duration::from_std(w).unwrap_or_else(|_| chrono::Duration::zero())
        });
        let arguments = match task_window_since {
            Some(since) => super::task_window::apply_window_args(&tool, arguments, since),
            None => arguments,
        };

        // Resolve the client through the mode-aware factory on every
        // call so a direct-mode toggle takes effect immediately
        // (#1710). The pre-baked-client variant of this code routed all
        // executions through the backend tinyhumans tenant regardless
        // of mode — silently breaking direct mode for tool execution.
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config = match config_rpc::reload_config_snapshot_with_timeout(
            self.config.as_ref(),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool execute.execute: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio_execute: failed to load live config: {e}"
                )));
            }
        };
        let kind = match create_composio_client(&live_config) {
            Ok(kind) => kind,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool execute.execute: factory failed");
                return Ok(ToolResult::error(format!("composio_execute failed: {e}")));
            }
        };

        let started = std::time::Instant::now();
        // Centralized prepare → retry → error-mapping pipeline (#1797),
        // mode-aware over the backend/direct split (#1710).
        let res = super::execute_dispatch::execute_composio_action_kind(
            kind,
            &tool,
            arguments,
            &live_config.composio.entity_id,
        )
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match res {
            Ok(resp) => {
                // Authoritative task-recency enforcement: drop task rows older
                // than the window. No-op unless a window is installed AND the
                // slug is a curated task-fetch action. Runs before the
                // markdown/JSON body decision so the agent reads filtered data.
                let resp = match task_window_since {
                    Some(since) => super::task_window::filter_response(&tool, resp, since),
                    None => resp,
                };
                tracing::info!(
                    tool = %tool,
                    successful = resp.successful,
                    error = ?resp.error,
                    elapsed_ms,
                    cost_usd = resp.cost_usd,
                    "[composio][execute] << result"
                );
                tracing::debug!(
                    tool = %tool,
                    response = ?resp,
                    "[composio][execute] << full response (DEBUG)"
                );
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                // Prefer the backend-rendered markdown when available
                // (tinyhumansai/backend#683). The backend handles parsing
                // for all composio actions; if a tool isn't formatted
                // server-side `markdown_formatted` is None and we fall
                // back to the raw JSON envelope.
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
                tracing::warn!(
                    tool = %tool,
                    error = %e,
                    elapsed_ms,
                    "[composio][execute] << dispatch error"
                );
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(e))
            }
        }
    }
}

// NOTE: A `composio_enable_scope` agent-callable meta-tool used to live
// here. It was removed deliberately: scope elevation is a
// security-sensitive, cross-session state change that unlocks
// destructive actions, and putting that flip behind LLM-mediated
// "user consent" both (a) made the safety contract depend on model
// behavior — the weakest place for it — and (b) was a soft gate the
// model could route around (e.g. trash-via-label). The user must
// toggle scopes themselves in **Connections → {toolkit} → {scope}
// row**; the agent only describes the gated capability and points at
// that UI path.
//
// Two surfaces name this policy; keep them in sync:
//   - `GatedIntegrationTool.unlock_paths` populated in `composio::ops`
//   - `scope_error_message` returned from `composio_execute` blocks

// ── Bulk registration helper ────────────────────────────────────────

/// Build the full set of composio agent tools when the integrations
/// client is available and composio is enabled. Returns an empty vec
/// otherwise so callers can always `.extend(...)` unconditionally.
pub fn all_composio_agent_tools(config: &crate::openhuman::config::Config) -> Vec<Box<dyn Tool>> {
    // Registration gate: ask the mode-aware probe "can this user call
    // composio at all?" — true when EITHER a backend session token OR a
    // stored/inline direct-mode API key is present. The pre-fix path
    // called `build_composio_client(...).is_none()`, which is
    // backend-only and silently dropped the 5 generic agent tools for
    // direct-mode users (#1710). Per-action dispatch inside each tool
    // re-resolves through the factory so the live `composio.mode`
    // toggle keeps winning.
    if !crate::openhuman::agent::harness::subagent_runner::user_is_signed_in_to_composio(config) {
        tracing::debug!(
            "[composio] agent tools not registered — user is not signed in to composio \
             (no backend session and no direct API key)"
        );
        return Vec::new();
    }
    // All five tools resolve their client per call through the
    // mode-aware factory; they only need a handle to the live root
    // config to do so. Sharing one `Arc<Config>` keeps the registration
    // cheap (no repeated `Config::clone` walks) and ensures every tool
    // sees the same live snapshot.
    let config_arc = Arc::new(config.clone());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(config_arc.clone())),
        Box::new(ComposioListConnectionsTool::new(config_arc.clone())),
        Box::new(ComposioAuthorizeTool::new(config_arc.clone())),
        // Inline-in-chat OAuth connect card (#3993). Raises an approval card
        // with a Connect button instead of handing the agent a raw URL.
        Box::new(ComposioConnectTool::new(config_arc.clone())),
        Box::new(ComposioListToolsTool::new(config_arc.clone())),
        Box::new(ComposioExecuteTool::new(config_arc)),
        // Pref-elevation is intentionally NOT an agent-callable tool;
        // the user must flip it themselves in the Connections UI.
        // See the long comment above the (removed) ComposioEnableScopeTool.
    ];
    tracing::debug!(count = tools.len(), "[composio] agent tools registered");
    tools
}
