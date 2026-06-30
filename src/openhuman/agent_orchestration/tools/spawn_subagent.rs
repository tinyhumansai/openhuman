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
//! Sub-agents always run in "typed" mode: a narrow archetype-specific
//! prompt with a filtered tool list, on a cheaper model where applicable.
//!
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunOutcome, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::memory_conversations::{
    self as conversations, ConversationMessage, CreateConversationThread,
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
                "description": "Sub-agent id (e.g. code_executor, researcher, critic)."
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
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this spawn only. Keeps the parent provider/routing, but pins the child agent to this model instead of the agent definition's default."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to — e.g. `gmail`, `notion`, `slack`. REQUIRED when `agent_id = \"integrations_agent\"`. Narrows the sub-agent's visible Composio actions AND its Connected Integrations prompt section to only that toolkit's catalogue, so the sub-agent's context window only carries the platform it was asked to operate on. Must match a currently-connected integration (see the Delegation Guide)."
                },
                "dedicated_thread": {
                    "type": "boolean",
                    "description": "Legacy compatibility flag. Delegations now always create a persistent worker thread when parent context is available, so this flag no longer gates thread creation."
                },
                "blocking": {
                    "type": "boolean",
                    "description": "Explicitly run the sub-agent inline and return its final output. Defaults to false; reusable async delegation is the default."
                },
                "task_key": {
                    "type": "string",
                    "description": "Optional deterministic identity key for reusable async delegation. Defaults to a normalized prompt/title."
                },
                "fresh": {
                    "type": "boolean",
                    "description": "When true, bypass reusable subagent matching and create a fresh durable worker."
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

        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Worker threads are now always created for delegations that may
        // need follow-up (checkpoint + replay for ask_user_clarification).
        // The `dedicated_thread` parameter is accepted but no longer
        // gates thread creation — every delegation gets a persistent
        // worker thread. (#3049 supersedes the #1624 disable.)
        let dedicated_thread = args
            .get("dedicated_thread")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let blocking = args
            .get("blocking")
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

        let definition = match registry.get(agent_id.as_str()) {
            Some(def) => def,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown agent_id '{agent_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        if let Some(parent_ctx) = current_parent() {
            if !parent_ctx.allowed_subagent_ids.contains(&definition.id) {
                log::warn!(
                    "[spawn_subagent] blocked subagent outside parent allowlist parent_agent={} requested_agent={} allowed={:?}",
                    parent_ctx.agent_definition_id,
                    definition.id,
                    parent_ctx.allowed_subagent_ids
                );
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: agent '{}' is not in parent agent '{}' subagents.allowlist",
                    definition.id, parent_ctx.agent_definition_id
                )));
            }
            log::debug!(
                "[spawn_subagent] subagent allowlist check passed parent_agent={} requested_agent={}",
                parent_ctx.agent_definition_id,
                definition.id
            );
        }

        // ── integrations_agent toolkit gate ──────────────────────────────────
        // integrations_agent is a platform-parameterised specialist. Every
        // spawn MUST name a CONNECTED toolkit so the sub-agent only
        // sees one integration's tool catalogue instead of all of
        // them. We split validation into three cases so the model
        // gets a precise, actionable error on every failure mode —
        // nothing reaches the LLM loop unless the spawn is valid.
        if definition.id == "integrations_agent" {
            // The parent's `connected_integrations` Vec is frozen at
            // session-start (see `session/turn.rs::fetch_connected_integrations`),
            // so a toolkit the user authorised mid-thread isn't visible
            // here. Refresh from the global integrations cache —
            // invalidated by `ComposioConnectionCreatedSubscriber` once
            // OAuth reaches ACTIVE — so the pre-flight sees the latest
            // truth. Falls back to the parent's frozen list when the
            // live fetch returns empty (no signed-in user, backend
            // unreachable, …) so offline behaviour is unchanged.
            let parent_ctx = current_parent();
            let live_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration> = {
                match crate::openhuman::config::Config::load_or_init().await {
                    Ok(config) => {
                        use crate::openhuman::composio::FetchConnectedIntegrationsStatus;
                        // Use the status-discriminating fetch so we can
                        // tell "user has zero active integrations" (truth
                        // — adopt it) apart from "backend unavailable"
                        // (preserve the parent's frozen snapshot so the
                        // pre-flight doesn't reject every toolkit during
                        // a transient 5xx).
                        match crate::openhuman::composio::fetch_connected_integrations_status(
                            &config,
                        )
                        .await
                        {
                            FetchConnectedIntegrationsStatus::Authoritative(fresh) => {
                                tracing::debug!(
                                    target: "spawn_subagent",
                                    count = fresh.len(),
                                    "[spawn_subagent] refreshed connected_integrations for pre-flight"
                                );
                                fresh
                            }
                            FetchConnectedIntegrationsStatus::Unavailable => {
                                tracing::debug!(
                                    target: "spawn_subagent",
                                    "[spawn_subagent] integrations backend unavailable; falling back to parent's frozen list"
                                );
                                parent_ctx
                                    .as_ref()
                                    .map(|p| p.connected_integrations.clone())
                                    .unwrap_or_default()
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            target: "spawn_subagent",
                            error = %e,
                            "[spawn_subagent] config load failed; falling back to parent's frozen list"
                        );
                        parent_ctx
                            .as_ref()
                            .map(|p| p.connected_integrations.clone())
                            .unwrap_or_default()
                    }
                }
            };
            let allowlist: Vec<&crate::openhuman::context::prompt::ConnectedIntegration> =
                live_integrations.iter().collect();
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
                            //
                            // Split (#2365) into 4 cases driven by the upstream
                            // status field on the most-informative connection
                            // row, instead of the legacy generic
                            // "not authorized yet" copy. Before this split,
                            // an OAuth-in-progress / expired / failed Gmail
                            // surfaced the same "you need to connect Gmail"
                            // message — which Settings UI contradicted (it
                            // shows the connection as initiated/expired), so
                            // users concluded the agent was confused.
                            tracing::debug!(
                                target: "spawn_subagent",
                                toolkit = %ci.toolkit,
                                non_active_status = ?ci.non_active_status,
                                "[spawn_subagent] integrations_agent gate: toolkit not connected — emitting status-specific message"
                            );
                            let message = describe_unconnected_state(
                                &ci.toolkit,
                                ci.non_active_status.as_deref(),
                            );
                            return Ok(ToolResult::success(message));
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

        if !blocking {
            let mut async_args = args;
            if let Some(obj) = async_args.as_object_mut() {
                obj.insert(
                    "agent_id".to_string(),
                    serde_json::Value::String(definition.id.clone()),
                );
                if obj.get("task_title").is_none() {
                    let title =
                        crate::openhuman::agent_orchestration::subagent_sessions::task_title_from_prompt(
                            &prompt,
                        );
                    obj.insert("task_title".to_string(), serde_json::Value::String(title));
                }
            }
            tracing::info!(
                target: "spawn_subagent",
                agent_id = %definition.id,
                "[spawn_subagent] routing to reusable async sub-agent by default"
            );
            return super::spawn_async_subagent::SpawnAsyncSubagentTool::new()
                .execute(async_args)
                .await;
        }

        // ── Publish SubagentSpawned event ──────────────────────────────
        let parent_session = current_parent()
            .map(|p| p.session_id.clone())
            .unwrap_or_else(|| "standalone".into());
        let task_id = format!("sub-{}", uuid::Uuid::new_v4());

        // Persist this delegation as a reopenable worker sub-thread, seeded
        // with the prompt, so the parent↔subagent conversation survives
        // navigation and restarts — the same machinery `spawn_worker_thread`
        // uses. Best-effort: with no parent context or thread store the run
        // still proceeds live-only (`worker_thread_id: None`).
        let worker_thread_id = current_parent().and_then(|p| {
            let parent_thread_id =
                crate::openhuman::inference::provider::thread_context::current_thread_id()?;
            let title: String = prompt.chars().take(60).collect();
            super::worker_thread::create_worker_thread(
                p.workspace_dir.clone(),
                &parent_thread_id,
                &definition.id,
                &title,
                &prompt,
            )
            .ok()
        });

        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: definition.id.clone(),
            mode: "typed".to_string(),
            task_id: task_id.clone(),
            prompt_chars: prompt.chars().count(),
        });

        // Mirror the spawn onto the parent's per-turn progress sink so the
        // web-channel bridge can stream a live subagent row into the
        // parent thread's UI. Best-effort: a closed/missing sink is
        // silently ignored — the global DomainEvent above is the
        // authoritative record.
        if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
            let _ = progress
                .send(AgentProgress::SubagentSpawned {
                    agent_id: definition.id.clone(),
                    task_id: task_id.clone(),
                    mode: "typed".to_string(),
                    dedicated_thread,
                    prompt_chars: prompt.chars().count(),
                    worker_thread_id: worker_thread_id.clone(),
                    display_name: Some(definition.display_name().to_string()),
                })
                .await;
        }

        // ── Run the sub-agent ──────────────────────────────────────────
        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override,
            context,
            model_override,
            task_id: Some(task_id.clone()),
            worker_thread_id: worker_thread_id.clone(),
            initial_history: None,
            checkpoint_dir: None,
            worktree_action_dir: None,
            run_queue: None,
        };

        let progress_sink = current_parent().and_then(|p| p.on_progress.clone());

        match run_subagent(definition, &prompt, options).await {
            Ok(outcome) => {
                match &outcome.status {
                    SubagentRunStatus::AwaitingUser {
                        question,
                        options: _,
                    } => {
                        // Sub-agent paused for user input — publish
                        // awaiting event and return structured envelope so
                        // the orchestrator can relay the question and later
                        // call continue_subagent.
                        publish_global(DomainEvent::SubagentAwaitingUser {
                            parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            question: question.clone(),
                        });
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentAwaitingUser {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    question: question.clone(),
                                    worker_thread_id: worker_thread_id.clone(),
                                })
                                .await;
                        }
                        let wt_display = worker_thread_id.as_deref().unwrap_or("(none)");
                        let envelope = format!(
                            "[SUBAGENT_AWAITING_USER]\n\
                             task_id: {}\n\
                             agent_id: {}\n\
                             worker_thread_id: {}\n\
                             question: {}\n\
                             [/SUBAGENT_AWAITING_USER]\n\n\
                             The sub-agent needs clarification before it can continue. \
                             Surface the above question to the user. When the user responds, \
                             call continue_subagent with the task_id, agent_id, and the \
                             user's answer as the message parameter.",
                            outcome.task_id, outcome.agent_id, wt_display, question,
                        );
                        Ok(ToolResult::success(envelope))
                    }
                    SubagentRunStatus::Completed => {
                        publish_global(DomainEvent::SubagentCompleted {
                            parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            output_chars: outcome.output.chars().count(),
                            iterations: outcome.iterations,
                        });

                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }

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
                                Ok(thread_id) => render_worker_thread_result(
                                    &thread_id,
                                    &definition.id,
                                    &outcome,
                                ),
                                Err(error) => {
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
                }
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
                    task_id: task_id.clone(),
                    agent_id: definition.id.clone(),
                    error: message.clone(),
                });

                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentFailed {
                            agent_id: definition.id.clone(),
                            task_id: task_id.clone(),
                            error: message.clone(),
                        })
                        .await;
                }
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
            parent_thread_id: None,
            labels: Some(vec!["tasks".to_string()]),
            personality_id: None,
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

/// Build the user-facing explanation for an allowlisted-but-not-active
/// integration during an `integrations_agent` spawn (#2365).
///
/// The single message that previously covered every cause ("available
/// but the user has not authorized it yet") looked confused to users
/// who had Gmail showing in Settings (because Settings reflects the
/// FE's optimistic post-OAuth view, while the spawn gate reads the
/// backend's authoritative status). We now pivot on the upstream
/// connection status:
///
/// - `INITIATED` / `INITIALIZING` / `PENDING` — OAuth in progress;
///   ask the user to finish the flow in their browser.
/// - `EXPIRED` — token rolled over; reconnect.
/// - `FAILED` / `ERROR` — handshake didn't land; reconnect.
/// - any other non-active status — quote the upstream verbatim.
/// - `None` — no connection row at all (truly disconnected).
///
/// Returns text the model reads literally; the orchestrator paraphrases
/// it into a user-facing reply. Keep the *intent* stable across
/// rewordings — the "Settings → Connections → {toolkit}" path is
/// load-bearing for the UI navigation tests.
pub(crate) fn describe_unconnected_state(toolkit: &str, status: Option<&str>) -> String {
    // Keep the original (trimmed) status separately so the
    // unknown-status branch can quote it verbatim — CodeRabbit
    // review on #2373: matching on the uppercased value AND
    // formatting with that uppercased value broke the
    // "quote upstream status verbatim" contract for mixed/lowercase
    // wire shapes.
    let trimmed = status.map(str::trim).filter(|s| !s.is_empty());
    let upper = trimmed.map(|s| s.to_ascii_uppercase());
    match upper.as_deref() {
        Some("INITIATED") | Some("INITIALIZING") | Some("PENDING") => format!(
            "Integration '{toolkit}' has an OAuth flow in progress but it hasn't reached \
             ACTIVE yet. Do NOT retry this spawn. Tell the user the authorization is \
             pending and ask them to finish the browser OAuth flow (Settings → \
             Connections → '{toolkit}') before retrying. If they already closed the \
             browser tab, they can restart the connection from the same Settings page."
        ),
        Some("EXPIRED") => format!(
            "Integration '{toolkit}' is connected but the OAuth token has expired. \
             Do NOT retry this spawn. Tell the user the connection expired and ask \
             them to reconnect '{toolkit}' at Settings → Connections → '{toolkit}' \
             before retrying the original request."
        ),
        Some("FAILED") | Some("ERROR") => {
            // Quote the actual upstream label (FAILED / ERROR) instead of
            // hard-coding "FAILED" — triage cross-references backend logs
            // and a misquoted `ERROR` row showing up as "FAILED" wastes
            // their time. graycyrus review on #2373.
            let raw = trimmed.unwrap_or("");
            format!(
                "Integration '{toolkit}' has a previous OAuth attempt in a `{raw}` state. \
                 Do NOT retry this spawn. Tell the user the connection failed and ask them \
                 to reconnect '{toolkit}' at Settings → Connections → '{toolkit}' before \
                 retrying the original request."
            )
        }
        Some(_) => {
            // Quote the *original* upstream status, not its uppercased
            // form — preserves "DeauthRequired" / "needs_relink"-style
            // mixed-case wire values for triage.
            let raw = trimmed.unwrap_or("");
            format!(
                "Integration '{toolkit}' has a connection row but its status is `{raw}`, \
                 which is not yet usable. Do NOT retry this spawn. Tell the user the \
                 connection is in an unusable state and ask them to reconnect '{toolkit}' \
                 at Settings → Connections → '{toolkit}'."
            )
        }
        _ => format!(
            "Integration '{toolkit}' is available but the user has not authorized it \
             yet. Do NOT retry this spawn. Tell the user the integration is available \
             and ask them to authorize '{toolkit}' in Settings → Connections → \
             '{toolkit}' before retrying the original request."
        ),
    }
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
            status: SubagentRunStatus::Completed,
            final_history: Vec::new(),
            usage: Default::default(),
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
    fn parameters_schema_advertises_optional_model_override() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").expect("schema has properties");
        let model = props.get("model").expect("model override advertised");
        assert_eq!(model.get("type").and_then(|v| v.as_str()), Some("string"));
        assert!(schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().all(|s| s.as_str() != Some("model")))
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
    fn persist_worker_thread_creates_thread_with_tasks_label_and_messages() {
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
        assert!(worker.labels.contains(&"tasks".to_string()));
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

    #[test]
    fn classify_subagent_failure_reframes_upstream_provider_outages() {
        let msg = SpawnSubagentTool::classify_subagent_failure(
            "provider call failed: all providers/models failed: upstream unavailable",
        );
        assert!(msg.contains("upstream inference unavailable"));
        assert!(msg.contains("NOT a Composio/integration auth issue"));
    }

    #[tokio::test]
    async fn dedicated_thread_flag_no_longer_returns_disabled_error() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "researcher",
                "prompt": "find x",
                "dedicated_thread": true,
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(!result.output().contains("temporarily disabled"));
    }

    #[tokio::test]
    async fn legacy_archetype_alias_is_accepted_for_lookup() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "archetype": "totally_made_up",
                "prompt": "x",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result
            .output()
            .contains("unknown agent_id 'totally_made_up'"));
    }

    #[tokio::test]
    async fn legacy_archetype_alias_is_forwarded_to_async_default_path() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "archetype": "researcher",
                "prompt": "research the reusable async default path",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result
                .output()
                .contains("spawn_async_subagent called outside of an agent turn"),
            "{}",
            result.output()
        );
        assert!(
            !result.output().contains("agent_id is required"),
            "{}",
            result.output()
        );
    }

    #[tokio::test]
    async fn integrations_agent_requires_toolkit_argument() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "integrations_agent",
                "prompt": "check gmail",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        let out = result.output();
        assert!(out.contains("`toolkit` argument is required"));
        assert!(out.contains("currently-connected toolkits"));
    }

    #[tokio::test]
    async fn integrations_agent_rejects_toolkit_outside_allowlist() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let toolkit = "totally_not_a_real_toolkit_slug";
        let result = tool
            .execute(json!({
                "agent_id": "integrations_agent",
                "prompt": "check gmail",
                "toolkit": toolkit,
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        let out = result.output();
        assert!(out.contains(&format!(
            "toolkit '{toolkit}' is not in the backend allowlist"
        )));
        assert!(out.contains("Valid toolkits"));
    }

    // ── #2365: describe_unconnected_state per upstream status ───────

    #[test]
    fn describe_unconnected_state_initiated_says_oauth_in_progress() {
        let msg = describe_unconnected_state("gmail", Some("INITIATED"));
        assert!(
            msg.contains("OAuth flow in progress"),
            "INITIATED must surface the in-progress wording: {msg}"
        );
        assert!(msg.contains("Settings → Connections → 'gmail'"));
        // The legacy "not authorized yet" copy must NOT leak into the
        // pending-OAuth branch — that was the user-perception bug
        // from #2365 (Settings UI showed Gmail connected, agent said
        // "not authorized").
        assert!(
            !msg.contains("has not authorized it yet"),
            "INITIATED must not borrow the truly-disconnected copy: {msg}"
        );
    }

    #[test]
    fn describe_unconnected_state_pending_and_initializing_are_aliased() {
        for status in ["PENDING", "INITIALIZING"] {
            let msg = describe_unconnected_state("gmail", Some(status));
            assert!(
                msg.contains("OAuth flow in progress"),
                "{status} must hit the in-progress branch: {msg}"
            );
        }
    }

    #[test]
    fn describe_unconnected_state_expired_says_reconnect() {
        let msg = describe_unconnected_state("gmail", Some("EXPIRED"));
        assert!(msg.contains("OAuth token has expired"));
        assert!(msg.contains("reconnect 'gmail'"));
        assert!(!msg.contains("OAuth flow in progress"));
    }

    #[test]
    fn describe_unconnected_state_failed_and_error_route_to_reconnect() {
        for status in ["FAILED", "ERROR"] {
            let msg = describe_unconnected_state("gmail", Some(status));
            let expected = format!("`{status}` state");
            assert!(
                msg.contains(&expected),
                "{status} must be quoted verbatim, not collapsed to a single label: {msg}"
            );
            assert!(msg.contains("reconnect 'gmail'"));
        }
    }

    #[test]
    fn describe_unconnected_state_failed_and_error_preserve_original_casing() {
        // Mixed-case wire values must round-trip through the FAILED /
        // ERROR branch with their original casing intact — that's the
        // whole point of graycyrus' review feedback.
        let lower_failed = describe_unconnected_state("gmail", Some("failed"));
        assert!(
            lower_failed.contains("`failed` state"),
            "lowercase `failed` must be quoted verbatim: {lower_failed}"
        );
        let mixed_error = describe_unconnected_state("gmail", Some("Error"));
        assert!(
            mixed_error.contains("`Error` state"),
            "mixed-case `Error` must be quoted verbatim: {mixed_error}"
        );
    }

    #[test]
    fn describe_unconnected_state_quotes_unknown_status_verbatim() {
        // Pin three shapes (uppercase / mixed / snake_case) so the
        // verbatim-quoting contract can't silently drift back to
        // echoing the matched (uppercased) value — that was the
        // CodeRabbit finding on #2373.
        for raw in ["DEAUTH_REQUIRED", "needs_relink", "PartialAuthRequired"] {
            let msg = describe_unconnected_state("gmail", Some(raw));
            let expected = format!("`{raw}`");
            assert!(
                msg.contains(&expected),
                "unknown status `{raw}` must be quoted verbatim (not its uppercased form): {msg}"
            );
        }
    }

    #[test]
    fn describe_unconnected_state_quotes_unknown_status_after_trimming_whitespace() {
        // Whitespace-only / blank statuses must NOT hit the
        // unknown-status branch — they collapse to the
        // truly-disconnected legacy copy via the `filter(|s|
        // !s.is_empty())` guard in `describe_unconnected_state`.
        let blank = describe_unconnected_state("gmail", Some("   "));
        assert!(
            blank.contains("has not authorized it yet"),
            "whitespace-only status must collapse to legacy None branch: {blank}"
        );
        // A real status with surrounding whitespace is quoted with
        // the whitespace trimmed (not preserved verbatim — triage
        // would not want padded backticks).
        let padded = describe_unconnected_state("gmail", Some("  DeauthRequired  "));
        assert!(
            padded.contains("`DeauthRequired`"),
            "trimmed status must be quoted in original casing: {padded}"
        );
    }

    #[test]
    fn describe_unconnected_state_none_is_truly_disconnected() {
        let msg = describe_unconnected_state("gmail", None);
        assert!(
            msg.contains("has not authorized it yet"),
            "None must hit the legacy never-connected copy: {msg}"
        );
        assert!(msg.contains("Settings → Connections → 'gmail'"));
    }

    #[test]
    fn describe_unconnected_state_status_match_is_case_insensitive() {
        // The status string flows in from Composio's wire format; we
        // can't assume casing. The classifier must normalise.
        let initiated = describe_unconnected_state("gmail", Some("initiated"));
        assert!(initiated.contains("OAuth flow in progress"));
        let expired = describe_unconnected_state("gmail", Some("Expired"));
        assert!(expired.contains("OAuth token has expired"));
    }
}
