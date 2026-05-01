//! Tool: `spawn_subagent` — delegate a sub-task to a specialised sub-agent.
//!
//! The orchestrator (or any parent agent that has this tool registered)
//! calls `spawn_subagent` to hand off a focused sub-task. The runner
//! looks up the requested [`AgentDefinition`] in the global registry,
//! filters the parent's tool registry per the definition, builds a
//! narrow system prompt, and runs an inner tool-call loop using the
//! parent's provider. The sub-agent's intra-loop history is collapsed
//! into a single text result that the parent receives as a normal
//! `tool_result`.
//!
//! Modes:
//! - `"typed"` (default) — narrow prompt + filtered tools + cheaper
//!   model. Use for delegated work where the parent doesn't need to
//!   share its full context.
//! - `"fork"` — replay the parent's *exact* rendered prompt + tool
//!   schemas + message prefix. Use for parallel decomposition of a
//!   homogeneous task; relies on the inference backend's automatic
//!   prefix caching for token savings.
//!
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunOutcome,
};
use crate::openhuman::memory::conversations::{
    self, ConversationMessage, CreateConversationThread,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Spawns a sub-agent of the requested type to handle a delegated task.
///
/// Registered into the parent agent's tool list by
/// [`crate::openhuman::tools::all_tools_with_runtime`]. The orchestrator
/// archetype's tool whitelist already includes `spawn_subagent`, so
/// orchestrated runs see it; non-orchestrator parents see it too unless
/// explicitly removed.
pub struct SpawnSubagentTool;

impl Default for SpawnSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self
    }

    fn classify_subagent_failure(message: &str) -> String {
        let lower = message.to_lowercase();
        let upstream_unhealthy = lower.contains("no healthy upstream")
            || lower.contains("upstream_unhealthy")
            || lower.contains("upstream unavailable")
            || lower.contains("service unavailable")
            || lower.contains("provider call failed: all providers/models failed");

        if upstream_unhealthy {
            return format!(
                "spawn_subagent failed: upstream inference unavailable \
                 (LLM provider outage/capacity). This is NOT a Composio/integration auth issue. \
                 Avoid immediate repeated retries; ask user to retry shortly.\nDetails: {message}"
            );
        }

        format!("spawn_subagent failed: {message}")
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Delegate a task to a specialised sub-agent only when direct \
         response or direct tools are insufficient. See the Delegation \
         Guide in the system prompt for available agent_ids and when to \
         use each. When delegating to `integrations_agent`, you MUST also pass \
         `toolkit=\"<name>\"` naming the Composio integration the \
         sub-task targets (e.g. `gmail`, `notion`); the sub-agent will \
         only see that toolkit's actions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // Build the agent_id enum dynamically from the global registry
        // when it's been initialised. Falls back to a string-with-hint
        // when the registry hasn't been set up yet (e.g. early tests).
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. code_executor, researcher, critic, fork)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": "Sub-agent id from the registry."
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": agent_id_schema,
                // Back-compat alias — older callers used `archetype`.
                "archetype": {
                    "type": "string",
                    "description": "Deprecated alias for `agent_id`. Use `agent_id` going forward."
                },
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the sub-agent. The sub-agent has no memory of the parent's conversation, so include all context the sub-agent needs to act."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to — e.g. `gmail`, `notion`, `slack`. REQUIRED when `agent_id = \"integrations_agent\"`. Narrows the sub-agent's visible Composio actions AND its Connected Integrations prompt section to only that toolkit's catalogue, so the sub-agent's context window only carries the platform it was asked to operate on. Must match a currently-connected integration (see the Delegation Guide)."
                },
                "mode": {
                    "type": "string",
                    "enum": ["typed", "fork"],
                    "description": "`typed` (default) builds a narrow prompt + filtered tools. `fork` replays the parent's exact prompt for prefix-cache reuse on the inference backend."
                },
                "dedicated_thread": {
                    "type": "boolean",
                    "description": "Default `false`. Set `true` ONLY for long, complex sub-tasks where the parent thread should not be flooded with sub-agent output. The sub-agent's prompt and final summary land in a fresh worker-labeled thread the user can open from the thread list, and the parent receives a compact reference (worker thread id + brief summary) instead of the full transcript. Worker threads cannot themselves spawn another worker (sub-agents never see this tool), so this is a one-level-deep escape hatch."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // ── Argument extraction with back-compat ───────────────────────
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("archetype").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("typed");

        let dedicated_thread = args
            .get("dedicated_thread")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // ── Validation ─────────────────────────────────────────────────
        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "spawn_subagent: `agent_id` (or legacy `archetype`) is required",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error("spawn_subagent: `prompt` is required"));
        }

        let registry = match AgentDefinitionRegistry::global() {
            Some(reg) => reg,
            None => {
                return Ok(ToolResult::error(
                    "spawn_subagent: AgentDefinitionRegistry has not been initialised. \
                     This usually means the core process started without calling \
                     AgentDefinitionRegistry::init_global at startup.",
                ));
            }
        };

        // Resolve `mode` against the definition. Explicit `mode` argument
        // wins; otherwise we infer from the definition itself.
        let lookup_id = if mode == "fork" {
            "fork"
        } else {
            agent_id.as_str()
        };
        let definition = match registry.get(lookup_id) {
            Some(def) => def,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown agent_id '{lookup_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        // ── integrations_agent toolkit gate ──────────────────────────────────
        // integrations_agent is a platform-parameterised specialist. Every
        // spawn MUST name a CONNECTED toolkit so the sub-agent only
        // sees one integration's tool catalogue instead of all of
        // them. We split validation into three cases so the model
        // gets a precise, actionable error on every failure mode —
        // nothing reaches the LLM loop unless the spawn is valid.
        if definition.id == "integrations_agent" {
            let parent_ctx = current_parent();
            let allowlist: Vec<&crate::openhuman::context::prompt::ConnectedIntegration> =
                parent_ctx
                    .as_ref()
                    .map(|p| p.connected_integrations.iter().collect())
                    .unwrap_or_default();
            let connected_slugs: Vec<String> = allowlist
                .iter()
                .filter(|ci| ci.connected)
                .map(|ci| ci.toolkit.clone())
                .collect();

            tracing::debug!(
                target: "spawn_subagent",
                toolkit = ?toolkit_override,
                allowlist_count = allowlist.len(),
                connected_count = connected_slugs.len(),
                connected = ?connected_slugs,
                "[spawn_subagent] integrations_agent gate: validating toolkit"
            );

            match toolkit_override.as_deref() {
                None => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent(integrations_agent): the `toolkit` argument is required. \
                         Pass one of the currently-connected toolkits: [{}]. \
                         See the Delegation Guide in your system prompt for which toolkit \
                         matches each task.",
                        connected_slugs.join(", ")
                    )));
                }
                Some(tk) => {
                    let entry = allowlist
                        .iter()
                        .find(|ci| ci.toolkit.eq_ignore_ascii_case(tk));
                    match entry {
                        None => {
                            // Toolkit isn't even in the backend allowlist.
                            return Ok(ToolResult::error(format!(
                                "spawn_subagent(integrations_agent): toolkit '{tk}' is not in \
                                 the backend allowlist. Valid toolkits: [{}]. Check the \
                                 Delegation Guide in your system prompt for the exact slug.",
                                allowlist
                                    .iter()
                                    .map(|ci| ci.toolkit.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )));
                        }
                        Some(ci) if !ci.connected => {
                            // Toolkit exists in the allowlist but isn't connected.
                            // This is NOT a tool error — it's an expected condition
                            // the orchestrator should communicate to the user. We
                            // return `ToolResult::success` so:
                            //   1. The agent loop doesn't prepend "Error: " to
                            //      the result text (which would bias the model
                            //      toward defensive failure language).
                            //   2. The web channel emits `success: true` on the
                            //      `tool_result` socket event, so the frontend
                            //      doesn't render this as a failed tool call.
                            // The model still reads the explanation and produces
                            // an appropriate user-facing response.
                            return Ok(ToolResult::success(format!(
                                "Integration '{tk}' is available but the user has not \
                                 authorized it yet. Do NOT retry this spawn. Tell the user \
                                 the integration is available and ask them to authorize \
                                 '{tk}' in Settings → Integrations before retrying the \
                                 original request."
                            )));
                        }
                        Some(_) => {
                            tracing::debug!(
                                target: "spawn_subagent",
                                toolkit = %tk,
                                "[spawn_subagent] integrations_agent gate: toolkit connected, proceeding with spawn"
                            );
                        }
                    }
                }
            }
        }

        // ── Publish SubagentSpawned event ──────────────────────────────
        let parent_session = current_parent()
            .map(|p| p.session_id.clone())
            .unwrap_or_else(|| "standalone".into());
        let task_id = format!("sub-{}", uuid::Uuid::new_v4());

        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: definition.id.clone(),
            mode: mode.to_string(),
            task_id: task_id.clone(),
            prompt_chars: prompt.chars().count(),
        });

        // ── Run the sub-agent ──────────────────────────────────────────
        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override,
            context,
            task_id: Some(task_id.clone()),
        };

        match run_subagent(definition, &prompt, options).await {
            Ok(outcome) => {
                publish_global(DomainEvent::SubagentCompleted {
                    parent_session,
                    task_id: outcome.task_id.clone(),
                    agent_id: outcome.agent_id.clone(),
                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                    output_chars: outcome.output.chars().count(),
                    iterations: outcome.iterations,
                });

                if dedicated_thread {
                    let workspace_dir = current_parent()
                        .map(|p| p.workspace_dir.clone())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let parent_visible = match persist_worker_thread(
                        &workspace_dir,
                        &definition.id,
                        &prompt,
                        &outcome,
                    ) {
                        Ok(thread_id) => {
                            render_worker_thread_result(&thread_id, &definition.id, &outcome)
                        }
                        Err(error) => {
                            // Persistence failure must not silently swallow the
                            // sub-agent's work — return the full output and
                            // surface the worker-thread error so the parent
                            // model can mention it. We deliberately fall
                            // through to a `success` ToolResult so the agent
                            // loop doesn't prepend "Error:" to text the
                            // sub-agent produced legitimately.
                            tracing::error!(
                                target: "spawn_subagent",
                                agent_id = %definition.id,
                                error = %error,
                                "[spawn_subagent] dedicated_thread persistence failed; \
                                 returning full sub-agent output inline"
                            );
                            format!(
                                "{}\n\n[worker_thread_error] failed to persist worker thread: {}",
                                outcome.output, error
                            )
                        }
                    };
                    return Ok(ToolResult::success(parent_visible));
                }

                Ok(ToolResult::success(outcome.output))
            }
            Err(err) => {
                let message = err.to_string();
                let parent_visible_error = Self::classify_subagent_failure(&message);
                // Log only non-sensitive context: agent_id and task_id. The raw
                // error message and classified summary may contain user prompts or
                // payload fragments — emit only a short type/kind indicator.
                let error_kind = message
                    .split(':')
                    .next()
                    .map(str::trim)
                    .unwrap_or("unknown");
                tracing::error!(
                    agent_id = %definition.id,
                    task_id = %task_id,
                    error_kind = %error_kind,
                    "[spawn_subagent] sub-agent execution failed"
                );
                publish_global(DomainEvent::SubagentFailed {
                    parent_session,
                    task_id,
                    agent_id: definition.id.clone(),
                    error: message.clone(),
                });
                // Surface as a non-fatal tool error so the parent model
                // can react and (e.g.) retry with different params.
                Ok(ToolResult::error(parent_visible_error))
            }
        }
    }
}

/// Trim a raw prompt down to a thread-list-friendly title.
///
/// Mirrors the visible-character cap the UI threads list uses so titles
/// stay readable when the orchestrator hands in a multi-paragraph prompt.
const WORKER_THREAD_TITLE_MAX_CHARS: usize = 80;

fn build_worker_thread_title(prompt: &str) -> String {
    let collapsed: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "Worker task".to_string();
    }
    let mut iter = collapsed.chars();
    let truncated: String = iter.by_ref().take(WORKER_THREAD_TITLE_MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn persist_worker_thread(
    workspace_dir: &std::path::Path,
    agent_id: &str,
    prompt: &str,
    outcome: &SubagentRunOutcome,
) -> Result<String, String> {
    let thread_id = format!("worker-{}", uuid::Uuid::new_v4());
    let title = build_worker_thread_title(prompt);
    let now = chrono::Utc::now().to_rfc3339();

    conversations::ensure_thread(
        workspace_dir.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title,
            created_at: now.clone(),
            labels: Some(vec!["worker".to_string()]),
        },
    )
    .map_err(|err| format!("ensure_thread: {err}"))?;

    conversations::append_message(
        workspace_dir.to_path_buf(),
        &thread_id,
        ConversationMessage {
            id: format!("user:{}", outcome.task_id),
            content: prompt.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "worker_thread",
                "agent_id": agent_id,
                "task_id": outcome.task_id,
            }),
            sender: "user".to_string(),
            created_at: now.clone(),
        },
    )
    .map_err(|err| format!("append user message: {err}"))?;

    conversations::append_message(
        workspace_dir.to_path_buf(),
        &thread_id,
        ConversationMessage {
            id: format!("agent:{}", outcome.task_id),
            content: outcome.output.clone(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "worker_thread",
                "agent_id": outcome.agent_id,
                "task_id": outcome.task_id,
                "elapsed_ms": outcome.elapsed.as_millis() as u64,
                "iterations": outcome.iterations,
            }),
            sender: "agent".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map_err(|err| format!("append agent message: {err}"))?;

    Ok(thread_id)
}

/// Build a parent-thread tool_result that refers the user to the worker
/// thread instead of dumping the sub-agent's full transcript inline.
///
/// The `[worker_thread_ref] … [/worker_thread_ref]` envelope carries
/// machine-readable metadata the UI parses to render a clickable card; the
/// surrounding prose stays informative for the LLM that reads the result.
fn render_worker_thread_result(
    thread_id: &str,
    agent_id: &str,
    outcome: &SubagentRunOutcome,
) -> String {
    let payload = json!({
        "thread_id": thread_id,
        "label": "worker",
        "agent_id": agent_id,
        "task_id": outcome.task_id,
        "elapsed_ms": outcome.elapsed.as_millis() as u64,
        "iterations": outcome.iterations,
    });
    format!(
        "Spawned worker thread `{thread_id}` for the delegated task. The \
         user can open it from the thread list (label: `worker`) to see \
         the sub-agent's full transcript. Continue from a brief summary \
         in this thread instead of relaying the entire run.\n\n\
         [worker_thread_ref]\n{payload}\n[/worker_thread_ref]",
        thread_id = thread_id,
        payload = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::subagent_runner::SubagentMode;
    use std::time::Duration;
    use tempfile::TempDir;

    fn sample_outcome(output: &str) -> SubagentRunOutcome {
        SubagentRunOutcome {
            agent_id: "researcher".into(),
            task_id: "sub-test-1".into(),
            output: output.to_string(),
            elapsed: Duration::from_millis(120),
            iterations: 3,
            mode: SubagentMode::Typed,
        }
    }

    #[test]
    fn build_worker_thread_title_collapses_whitespace_and_caps_length() {
        let prompt = "  draft\n a very long\tplan that\nrambles ".to_string() + &"x".repeat(200);
        let title = build_worker_thread_title(&prompt);
        assert!(title.starts_with("draft a very long plan"));
        assert!(title.chars().count() <= WORKER_THREAD_TITLE_MAX_CHARS + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn build_worker_thread_title_falls_back_when_empty() {
        assert_eq!(build_worker_thread_title("   \n\t  "), "Worker task");
    }

    #[test]
    fn parameters_schema_advertises_dedicated_thread_flag() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").expect("schema has properties");
        let flag = props
            .get("dedicated_thread")
            .expect("dedicated_thread advertised");
        assert_eq!(flag.get("type").and_then(|v| v.as_str()), Some("boolean"));
        // Must be off by default — workers are an opt-in escape hatch, not
        // a free upgrade for every spawn.
        assert!(schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().all(|s| s.as_str() != Some("dedicated_thread")))
            .unwrap_or(true));
    }

    #[test]
    fn render_worker_thread_result_carries_machine_readable_envelope() {
        let outcome = sample_outcome("done");
        let rendered = render_worker_thread_result("worker-abc", "researcher", &outcome);
        assert!(rendered.contains("Spawned worker thread `worker-abc`"));
        assert!(rendered.contains("[worker_thread_ref]"));
        assert!(rendered.contains("[/worker_thread_ref]"));
        // The JSON payload between the markers must round-trip.
        let start = rendered.find("[worker_thread_ref]\n").unwrap() + "[worker_thread_ref]\n".len();
        let end = rendered.find("\n[/worker_thread_ref]").unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&rendered[start..end]).expect("valid json envelope");
        assert_eq!(payload["thread_id"], "worker-abc");
        assert_eq!(payload["label"], "worker");
        assert_eq!(payload["agent_id"], "researcher");
        assert_eq!(payload["task_id"], "sub-test-1");
        assert_eq!(payload["iterations"], 3);
    }

    #[test]
    fn persist_worker_thread_creates_thread_with_worker_label_and_messages() {
        let temp = TempDir::new().expect("tempdir");
        let outcome = sample_outcome("the answer is 42");
        let thread_id = persist_worker_thread(
            temp.path(),
            "researcher",
            "draft a long research plan",
            &outcome,
        )
        .expect("worker thread persisted");

        assert!(thread_id.starts_with("worker-"));

        let threads = conversations::list_threads(temp.path().to_path_buf()).expect("list threads");
        let worker = threads
            .iter()
            .find(|t| t.id == thread_id)
            .expect("worker thread present");
        assert!(worker.labels.contains(&"worker".to_string()));
        assert!(worker.title.starts_with("draft a long research plan"));

        let messages =
            conversations::get_messages(temp.path().to_path_buf(), &thread_id).expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].sender, "user");
        assert_eq!(messages[0].content, "draft a long research plan");
        assert_eq!(messages[1].sender, "agent");
        assert_eq!(messages[1].content, "the answer is 42");
        assert_eq!(messages[1].extra_metadata["iterations"], 3);
        assert_eq!(messages[1].extra_metadata["scope"], "worker_thread");
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "prompt": "do thing"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("agent_id"));
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "researcher"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("prompt"));
    }

    #[tokio::test]
    async fn no_registry_returns_clear_error() {
        // The global registry has not been initialised in this test.
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "researcher",
                "prompt": "find x",
            }))
            .await
            .unwrap();
        // Either: registry uninitialised → clear init error, OR
        // registry was initialised by a previous test → "no parent context"
        // because we're not running inside an Agent::turn. Both are
        // acceptable: the tool gracefully refuses.
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn unknown_agent_id_lists_available() {
        // Force-init the global registry with builtins.
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "totally_made_up",
                "prompt": "x",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        let out = result.output();
        // Should list at least one valid built-in.
        assert!(out.contains("code_executor") || out.contains("researcher"));
    }
}
