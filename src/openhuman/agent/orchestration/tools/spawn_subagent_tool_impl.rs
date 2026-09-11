#[async_trait]
impl Tool for SpawnSubagentTool {

    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Delegate a task to a specialised sub-agent only when direct \
         response or direct tools are insufficient. Handles ONE delegated task \
         per call: by default it runs as a reusable async worker and returns \
         immediately — pass `blocking: true` to run it inline and get the \
         sub-agent's final output back in this turn. To run several independent \
         workers at once (e.g. \"a separate researcher for each X\", a council \
         of opinions, or \"fan out over N items\"), use `spawn_parallel_agents` \
         with one task per worker — a SINGLE call that launches them \
         concurrently. Do NOT call this tool in a loop to fan out: repeated \
         `spawn_subagent` calls each delegate a single task and never launch \
         workers concurrently, which serializes the whole request. See the Delegation \
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
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
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
            let live_integrations: Vec<
                crate::openhuman::agent::context::prompt::ConnectedIntegration,
            > = {
                match crate::openhuman::config::Config::load_or_init().await {
                    Ok(config) => {
                        use crate::openhuman::integrations::composio::FetchConnectedIntegrationsStatus;
                        // Use the status-discriminating fetch so we can
                        // tell "user has zero active integrations" (truth
                        // — adopt it) apart from "backend unavailable"
                        // (preserve the parent's frozen snapshot so the
                        // pre-flight doesn't reject every toolkit during
                        // a transient 5xx).
                        match crate::openhuman::integrations::composio::fetch_connected_integrations_status(
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
            let allowlist: Vec<&crate::openhuman::agent::context::prompt::ConnectedIntegration> =
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

        // Async-by-default only holds where the finished result has somewhere
        // to land. `spawn_async_subagent` delivers thread-addressed (see
        // `background_delivery`), so outside a chat turn (flow `agent` node,
        // CLI, cron) it now refuses outright (B40). Self-heal to blocking
        // dispatch here rather than forwarding into that guard: the caller
        // asked to delegate, and running the sub-agent inline is the one mode
        // that both executes it and returns its output. Mirrors the
        // `has_delivery_thread` fallback the `delegate_*` tools already do in
        // `dispatch.rs::dispatch_subagent`.
        let has_delivery_thread =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id().is_some();
        if !blocking && !has_delivery_thread {
            log::info!(
                "[spawn_subagent] async delegation requested for '{}' but no delivery thread \
                 (flow node / CLI / cron context) — falling back to blocking dispatch",
                definition.id
            );
        }
        if !blocking && has_delivery_thread {
            let mut async_args = args;
            if let Some(obj) = async_args.as_object_mut() {
                obj.insert(
                    "agent_id".to_string(),
                    serde_json::Value::String(definition.id.clone()),
                );
                if obj.get("task_title").is_none() {
                    let title =
                        crate::openhuman::agent::orchestration::subagent_sessions::task_title_from_prompt(
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
                .execute_with_context(async_args, ToolCallOptions::default(), tool_context)
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
                crate::openhuman::agent::tinyagents::thread_context::current_thread_id()?;
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

        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            definition.id.clone(),
            "typed".to_string(),
            task_id.clone(),
            prompt.chars().count(),
        );

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
                    prompt: prompt.clone(),
                    worker_thread_id: worker_thread_id.clone(),
                    display_name: Some(definition.display_name().to_string()),
                })
                .await;
        }

        // ── Run the sub-agent ──────────────────────────────────────────
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace().cloned());
        let worktree_action_dir = workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        if let Some(descriptor) = workspace_descriptor.as_ref() {
            tracing::debug!(
                task_id = %task_id,
                agent_id = %definition.id,
                workspace_root = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[spawn_subagent] using ToolExecutionContext workspace root"
            );
        }
        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override,
            context,
            model_override,
            task_id: Some(task_id.clone()),
            worker_thread_id: worker_thread_id.clone(),
            initial_history: None,
            checkpoint_dir: None,
            worktree_action_dir,
            workspace_descriptor,
            run_queue: None,
        };

        let progress_sink = current_parent().and_then(|p| p.on_progress.clone());

        match run_subagent(definition, &prompt, options).await {
            Ok(outcome) => {
                match &outcome.status {
                    SubagentRunStatus::AwaitingUser {
                        question,
                        options: _,
                        checkpoint,
                    } => {
                        // Sub-agent paused for user input — publish
                        // awaiting event and return structured envelope so
                        // the orchestrator can relay the question and later
                        // call continue_subagent.
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            question.clone(),
                        );
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentAwaitingUser {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    question: question.clone(),
                                    worker_thread_id: worker_thread_id.clone(),
                                    checkpoint_path: checkpoint
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().to_string()),
                                })
                                .await;
                        }
                        let envelope = super::awaiting_user::awaiting_user_envelope(
                            &outcome.task_id,
                            &outcome.agent_id,
                            worker_thread_id.as_deref(),
                            question,
                            checkpoint.is_some(),
                        );
                        Ok(ToolResult::success(envelope))
                    }
                    SubagentRunStatus::Completed => {
                        // #3883: log the orchestrator taking delivery of each
                        // artifact path the child handed back, so a run journal
                        // shows both ends of every `[artifact]` pointer. The
                        // `consumed_by_parent` stage distinguishes this from the
                        // child's `recorded_by_child` line for the same path.
                        crate::openhuman::agent::harness::artifact_offload::note_artifact_handoff(
                            crate::openhuman::agent::harness::artifact_offload::HANDOFF_STAGE_CONSUMED,
                            &outcome.agent_id,
                            &outcome.task_id,
                            &outcome.artifact_paths,
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );

                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    output: outcome.output.clone(),
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
                    SubagentRunStatus::Incomplete { reason } => {
                        // The sub-agent stopped WITHOUT reaching its goal (a
                        // no-progress circuit breaker halted it, or it hit the
                        // iteration cap). Hand the orchestrator a structured
                        // envelope carrying BOTH the blocker and the partial
                        // progress — NOT the "nothing happened" failure envelope
                        // (work WAS done) and NOT a bare success it would narrate
                        // as done or re-spin (#4096).
                        tracing::info!(
                            agent_id = %outcome.agent_id,
                            task_id = %outcome.task_id,
                            iterations = outcome.iterations,
                            "[spawn_subagent] sub-agent stopped incomplete — returning structured handback"
                        );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                        if let Some(ref tx) = progress_sink {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id.clone(),
                                    task_id: outcome.task_id.clone(),
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                    output: outcome.output.clone(),
                                    worktree_path: None,
                                    changed_files: Vec::new(),
                                    dirty_status: None,
                                })
                                .await;
                        }
                        let envelope = format!(
                            "[SUBAGENT_INCOMPLETE]\n\
                             task_id: {}\n\
                             agent_id: {}\n\
                             reason: the sub-agent {reason}\n\
                             progress:\n{}\n\
                             [/SUBAGENT_INCOMPLETE]\n\n\
                             The sub-agent did NOT finish. Above is the partial progress it \
                             made. Do NOT report this as done or fabricate a result. Decide: \
                             relay the partial result and the blocker to the user, continue with \
                             a different approach, or escalate — but do not re-run the identical \
                             delegation unchanged.",
                            outcome.task_id, outcome.agent_id, outcome.output,
                        );
                        Ok(ToolResult::success(envelope))
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
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                    parent_session,
                    task_id.clone(),
                    definition.id.clone(),
                    message.clone(),
                );

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
