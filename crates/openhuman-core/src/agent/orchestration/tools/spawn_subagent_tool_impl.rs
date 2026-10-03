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
         workers at once (e.g. \"a separate worker for each X\", a council \
         of opinions, or \"fan out over N items\"), use `spawn_parallel_agents` \
         with one task per worker — a SINGLE call that launches them \
         concurrently. Do NOT call this tool in a loop to fan out: repeated \
         `spawn_subagent` calls each delegate a single task and never launch \
         workers concurrently, which serializes the whole request. See the Delegation \
         Guide in the system prompt for available agent_ids and when to \
         use each."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        spawn_subagent_parameters_schema()
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
        if let Some(live_parent) = super::ambient_parent_run_context("direct-spawn-subagent") {
            let run_context = live_parent.data.child();
            return self
                .execute_with_live_parent_context(
                    args,
                    tool_context,
                    run_context,
                    Some(&live_parent),
                )
                .await;
        }
        self.execute_with_parent_context(
            args,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        )
        .await
    }
}

impl SpawnSubagentTool {
    pub(crate) async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_live_parent_context(args, tool_context, run_context, None)
            .await
    }

    pub(crate) async fn execute_with_live_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        live_parent: Option<
            &tinyagents_harness::context::RunContext<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
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

        // Harness registry first, then an enabled custom agent in the
        // session's config — a user-authored sub-agent lives only in the
        // latter (#6934).
        let config = run_context
            .parent
            .as_ref()
            .and_then(|parent| parent.runtime_config.as_deref());
        let definition = match crate::agent::registry::resolve_spawnable_definition(
            registry,
            config,
            agent_id.as_str(),
        ) {
            Some(def) => def,
            None => {
                let available = crate::agent::registry::spawnable_ids(registry, config);
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown agent_id '{agent_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        if let Some(parent_ctx) = run_context.parent.as_ref() {
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

        // Input, registry, allowlist, and integration validation are safe to
        // perform without a live run. A valid spawn must still fail closed
        // unless its typed harness parent carries authority and cancellation.
        let Some(live_parent) = live_parent else {
            return Ok(ToolResult::error(
                "spawn_subagent requires a live harness run context.",
            ));
        };

        // Async-by-default only holds where the finished result has somewhere
        // to land. `spawn_async_subagent` delivers thread-addressed (see
        // `background_delivery`), so outside a chat turn (flow `agent` node,
        // CLI, cron) it now refuses outright (B40). Self-heal to blocking
        // dispatch here rather than forwarding into that guard: the caller
        // asked to delegate, and running the sub-agent inline is the one mode
        // that both executes it and returns its output. Mirrors the
        // `has_delivery_thread` fallback the `delegate_*` tools already do in
        // `dispatch.rs::dispatch_subagent`.
        let parent_thread_id = tool_context
            .and_then(ToolRunContext::thread_id)
            .or(run_context.thread_id.as_deref())
            .map(str::to_owned);
        let has_delivery_thread = parent_thread_id.is_some();
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
                        crate::agent::orchestration::subagent_sessions::task_title_from_prompt(
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
            let detached_data = live_parent.data.detached_child();
            let detached_cancellation = detached_data.cancellation.clone();
            let detached_parent = live_parent
                .child(
                    tinyagents_harness::context::RunConfig::new(format!(
                        "async-subagent-{}",
                        uuid::Uuid::new_v4()
                    )),
                    detached_data,
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .with_cancellation(detached_cancellation);
            return super::spawn_async_subagent::SpawnAsyncSubagentTool::new()
                .execute_with_live_parent_context(
                    async_args,
                    tool_context,
                    run_context,
                    detached_parent,
                )
                .await;
        }

        // ── Publish SubagentSpawned event ──────────────────────────────
        let parent_session = run_context
            .parent
            .as_ref()
            .map(|p| p.session_id.clone())
            .unwrap_or_else(|| "standalone".into());
        let task_id = format!("sub-{}", uuid::Uuid::new_v4());

        // Persist this delegation as a reopenable worker sub-thread, seeded
        // with the prompt, so the parent↔subagent conversation survives
        // navigation and restarts — the same machinery `spawn_worker_thread`
        // uses. Best-effort: with no parent context or thread store the run
        // still proceeds live-only (`worker_thread_id: None`).
        let worker_thread_id = run_context.parent.as_ref().and_then(|p| {
            let parent_thread_id = parent_thread_id.as_ref()?;
            let title: String = prompt.chars().take(60).collect();
            super::worker_thread::create_worker_thread(
                p.workspace_dir.clone(),
                parent_thread_id,
                &definition.id,
                &title,
                &prompt,
            )
            .ok()
        });

        crate::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            definition.id.clone(),
            "typed".to_string(),
            task_id.clone(),
            prompt.chars().count(),
        );

        if let Some(progress) = run_context.progress.clone() {
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
                    parent_call_id: crate::tools::host_extensions::tool_call_id(tool_context),
                })
                .await;
        }

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
        let progress_sink = run_context.progress.clone();
        let parent_workspace_dir = run_context
            .parent
            .as_ref()
            .map(|parent| parent.workspace_dir.clone());
        let options = SubagentRunOptions {
            skill_filter_override: None,
            context,
            model_override,
            task_id: Some(task_id.clone()),
            thread_id: parent_thread_id,
            run_context,
            worker_thread_id: worker_thread_id.clone(),
            initial_history: None,
            checkpoint_dir: None,
            worktree_action_dir,
            workspace_descriptor,
            run_queue: None,
        };

        let run =
            run_subagent_with_parent(live_parent, definition.clone(), prompt.clone(), options)
                .await;
        match run {
            Ok(outcome) => {
                let emit_lifecycle_effects = outcome.should_emit_lifecycle_effects();
                match &outcome.status {
                    SubagentRunStatus::AwaitingUser {
                        question,
                        options: _,
                        checkpoint,
                    } => {
                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
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
                        crate::agent::harness::artifact_offload::note_artifact_handoff(
                            crate::agent::harness::artifact_offload::HANDOFF_STAGE_CONSUMED,
                            &outcome.agent_id,
                            &outcome.task_id,
                            &outcome.artifact_paths,
                        );
                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
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
                                        // BLOCKING spawn: this child's usage DID reach the parent's
                                        // ledger, so `chat_done` already carries its tokens AND its
                                        // cost. Populating here would double both. See the field's docs.
                                        usage: None,
                                        worktree_path: None,
                                        changed_files: Vec::new(),
                                        dirty_status: None,
                                    })
                                    .await;
                            }
                        }

                        if dedicated_thread {
                            let workspace_dir = parent_workspace_dir
                                .clone()
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
                        if emit_lifecycle_effects {
                            crate::agent::orchestration::subagent_events::publish_subagent_completed(
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
                                        // BLOCKING spawn: this child's usage DID reach the parent's
                                        // ledger, so `chat_done` already carries its tokens AND its
                                        // cost. Populating here would double both. See the field's docs.
                                        usage: None,
                                        worktree_path: None,
                                        changed_files: Vec::new(),
                                        dirty_status: None,
                                    })
                                    .await;
                            }
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
                    SubagentRunStatus::Cancelled => {
                        tracing::info!(
                            agent_id = %outcome.agent_id,
                            task_id = %outcome.task_id,
                            "[spawn_subagent] sub-agent cancelled"
                        );
                        if emit_lifecycle_effects {
                            let message = "sub-agent was cancelled".to_string();
                            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                                parent_session,
                                outcome.task_id.clone(),
                                outcome.agent_id.clone(),
                                message.clone(),
                            );
                            if let Some(ref tx) = progress_sink {
                                let _ = tx
                                    .send(AgentProgress::SubagentFailed {
                                        agent_id: outcome.agent_id.clone(),
                                        task_id: outcome.task_id.clone(),
                                        error: message,
                                    })
                                    .await;
                            }
                        }
                        Ok(ToolResult::error(
                            "spawn_subagent: delegated sub-agent was cancelled",
                        ))
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
                crate::agent::orchestration::subagent_events::publish_subagent_failed(
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
