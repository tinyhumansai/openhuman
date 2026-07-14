//! The first-class `rhai_workflows` tool — the orchestrator's language-workflow surface.
//!
//! One tool call evaluates one Rhai cell against a persistent session
//! namespace. All effectful work a cell performs goes through the bridged inner
//! tools/models/sub-agents (each carrying its own approval/permission gates in
//! [`super::bridge`]), so the `rhai_workflows` tool itself declares no external effect.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope, ToolTimeout};

use super::policy::DEFAULT_RHAI_TIMEOUT_SECS;
use super::types::RhaiEvalRequest;

/// The `rhai_workflows` language-workflow tool. Stateless: it resolves the parent turn
/// context, autonomy tier, and cancellation per call.
pub struct RhaiTool;

impl RhaiTool {
    /// Builds the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RhaiTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RhaiTool {
    fn name(&self) -> &str {
        "rhai_workflows"
    }

    fn description(&self) -> &str {
        "Author and run a small Rhai workflow *cell* to orchestrate ad-hoc control flow — \
fan-out, loops, dedup/verify pipelines — that fixed spawn/parallel tools cannot express. \
One call evaluates one cell; top-level `let` bindings persist within a `session_id` so a \
later cell continues the same namespace.\n\
\n\
In-cell built-ins:\n\
• tool_call(#{tool, arguments}) — call one of your visible tools; returns its content.\n\
• agent_query(#{agent, prompt}) — spawn a sub-agent (by id from your allowlist); returns its text.\n\
• model_query(#{model, system?, prompt?}) — one model completion; returns the text.\n\
• tool_call_batched([...]) / agent_query_batched([...]) / model_query_batched([...]) — bounded-concurrency fan-out; each returns an array aligned with its input.\n\
• emit(name, #{..}) — record a progress event. answer(text) — set the cell's final answer.\n\
• print(x) — captured into stdout. Ordinary Rhai: let, if, for/while, arrays #[..], maps #{..}.\n\
\n\
Bounds (fail-closed): per-cell wall-clock timeout, and per-session caps on model/tool/agent \
calls, iterations, output bytes, and recursion depth. Exceeding one returns an error you can \
fix and retry in the same session. Excluded from cells: `rhai_workflows` itself, `spawn_*`, and workflow tools.\n\
\n\
Example — parallel fan-out over sub-agents, then reduce:\n\
```\n\
let topics = [\"auth\", \"billing\", \"search\"];\n\
let findings = agent_query_batched(topics.map(|t| #{ agent: \"researcher\", prompt: `Investigate ${t}` }));\n\
for f in findings { print(f); }\n\
answer(`Reviewed ${findings.len()} areas`);\n\
```\n\
\n\
Example — batched tool calls, keep only the hits:\n\
```\n\
let files = [\"a.rs\", \"b.rs\", \"c.rs\"];\n\
let reads = tool_call_batched(files.map(|p| #{ tool: \"read_file\", arguments: #{ path: p } }));\n\
let hits = [];\n\
for r in reads { if r.ok && r.content.contains(\"TODO\") { hits.push(r.content); } }\n\
hits\n\
```\n\
\n\
Prefer `rhai_workflows` over spawn_parallel_agents when you need loops, conditionals, or a dedup/verify \
pipeline over results. Pass a `session_id` to continue a prior cell's bindings; set \
`close_session: true` when done."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Rhai workflow cell to evaluate. Top-level `let` bindings persist within a session_id."
                },
                "session_id": {
                    "type": "string",
                    "description": "Continue a prior Rhai session's namespace; omit for a fresh session (its generated id is returned)."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 3600,
                    "description": "Per-cell wall-clock timeout in seconds (default 300)."
                },
                "limits": {
                    "type": "object",
                    "description": "Optional per-session limit overrides (clamped; only the `full` autonomy tier may raise above defaults).",
                    "properties": {
                        "max_tool_calls": { "type": "integer" },
                        "max_agent_calls": { "type": "integer" },
                        "max_model_calls": { "type": "integer" },
                        "max_concurrency": { "type": "integer" }
                    }
                },
                "close_session": {
                    "type": "boolean",
                    "description": "Close (drop) the session after this cell."
                }
            },
            "required": ["script"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Matches spawn_subagent: an orchestration primitive that can drive
        // effectful inner tools (each of which re-gates itself).
        PermissionLevel::Execute
    }

    fn scope(&self) -> ToolScope {
        // Orchestrator surface only; not exposed over CLI/RPC in v1.
        ToolScope::AgentOnly
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        // Always an explicit bound (never Inherit/Unbounded): a legitimate
        // fan-out outlives the default inherit budget, but must still be capped.
        let requested = args.get("timeout_secs").and_then(Value::as_u64);
        let secs = crate::openhuman::tool_timeout::explicit_call_timeout_secs(
            requested,
            crate::openhuman::tool_timeout::MAX_TIMEOUT_SECS,
        )
        .unwrap_or(DEFAULT_RHAI_TIMEOUT_SECS);
        ToolTimeout::Secs(secs)
    }

    fn display_label(&self, _args: &Value) -> Option<String> {
        Some("running Rhai workflow".to_string())
    }

    fn display_detail(&self, args: &Value) -> Option<String> {
        let script = args.get("script").and_then(Value::as_str)?;
        let first = script
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim();
        if first.chars().count() > 80 {
            Some(format!("{}…", first.chars().take(79).collect::<String>()))
        } else {
            Some(first.to_string())
        }
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let req: RhaiEvalRequest = match serde_json::from_value(args) {
            Ok(req) => req,
            Err(err) => {
                return Ok(ToolResult::error(format!(
                    "rhai_workflows: invalid arguments: {err}. Required: `script` (string). \
                     Optional: session_id, timeout_secs (1–3600), limits, close_session."
                )));
            }
        };
        if req.script.trim().is_empty() {
            return Ok(ToolResult::error(
                "rhai_workflows: `script` is required and must be a non-empty Rhai cell.",
            ));
        }

        match super::ops::eval_rhai_cell(req).await {
            Ok(response) => match serde_json::to_value(&response) {
                Ok(value) => Ok(ToolResult::json(value)),
                Err(err) => Ok(ToolResult::error(format!(
                    "rhai_workflows: internal error rendering result: {err}"
                ))),
            },
            Err(err) => {
                log::info!(
                    "[rhai_workflows] tool returning error result (kind={})",
                    err.kind()
                );
                Ok(ToolResult::error(err.message()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = RhaiTool::new();
        assert_eq!(tool.name(), "rhai_workflows");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        assert!(matches!(tool.scope(), ToolScope::AgentOnly));
        assert!(!tool.external_effect());
    }

    #[test]
    fn timeout_is_always_bounded() {
        let tool = RhaiTool::new();
        assert_eq!(
            tool.timeout_policy(&json!({})),
            ToolTimeout::Secs(DEFAULT_RHAI_TIMEOUT_SECS)
        );
        assert_eq!(
            tool.timeout_policy(&json!({ "timeout_secs": 42 })),
            ToolTimeout::Secs(42)
        );
        // Out-of-range requests are clamped, never unbounded.
        assert_eq!(
            tool.timeout_policy(&json!({ "timeout_secs": 100000 })),
            ToolTimeout::Secs(3600)
        );
    }

    #[test]
    fn display_detail_elides_first_script_line() {
        let tool = RhaiTool::new();
        let detail = tool
            .display_detail(&json!({ "script": "\n\nlet x = 1;\nlet y = 2;" }))
            .expect("detail");
        assert_eq!(detail, "let x = 1;");
    }

    #[tokio::test]
    async fn empty_script_is_a_model_consumable_error() {
        let tool = RhaiTool::new();
        let result = tool
            .execute(json!({ "script": "   " }))
            .await
            .expect("ok result");
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn missing_script_is_a_model_consumable_error() {
        let tool = RhaiTool::new();
        // No `script` field at all → invalid arguments, not a panic.
        let result = tool
            .execute(json!({ "session_id": "x" }))
            .await
            .expect("ok result");
        assert!(result.is_error);
        assert!(result.output_for_llm(false).contains("script"));
    }

    #[test]
    fn schema_requires_script() {
        let schema = RhaiTool::new().parameters_schema();
        assert_eq!(schema["required"], json!(["script"]));
    }
}
