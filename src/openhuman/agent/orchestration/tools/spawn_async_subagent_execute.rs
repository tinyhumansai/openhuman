impl SpawnAsyncSubagentTool {
    async fn execute_with_context_inner(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
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
            .map(str::to_string);
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
        let task_title = args
            .get("task_title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Background subagent")
            .to_string();
        let task_key_source = durable_task_key_source(&args, &prompt, context.as_deref());
        let task_key = subagent_sessions::normalize_task_key(&task_key_source);
        let force_fresh = args.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);

        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `agent_id` is required",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `prompt` is required",
            ));
        }

        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent called outside of an agent turn",
                ));
            }
        };

        let registry = match AgentDefinitionRegistry::global() {
            Some(registry) => registry,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent: AgentDefinitionRegistry has not been initialised",
                ));
            }
        };
        let definition = match registry.get(&agent_id).cloned() {
            Some(definition) => definition,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_async_subagent: unknown agent_id '{agent_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        if !parent.allowed_subagent_ids.contains(&definition.id) {
            log::warn!(
                "[spawn_async_subagent] blocked subagent outside allowlist parent={} requested={} allowed={:?}",
                parent.agent_definition_id,
                definition.id,
                parent.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "spawn_async_subagent: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, parent.agent_definition_id
            )));
        }

        if definition.id == "integrations_agent" && toolkit_override.is_none() {
            return Ok(ToolResult::error(
                "spawn_async_subagent(integrations_agent): the `toolkit` argument is required",
            ));
        }

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();
        let parent_thread_id =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id();

        // Async delivery is thread-addressed: the finished result is inserted
        // back into the parent chat thread as a follow-up turn
        // (`background_delivery`). Outside a chat turn (flow `agent` nodes,
        // CLI, cron) there is no `current_thread_id()` to deliver into, so
        // `background_delivery::deliver_batch` logs "dropping headless batch"
        // and the (possibly real, completed) work is silently discarded — the
        // caller sees "Accepted" and never learns the result never arrived.
        // Fail loudly instead: the caller has a synchronous alternative
        // (`spawn_subagent` with `blocking: true`, or a `delegate_*` tool).
        // Both of those self-heal to blocking dispatch in this situation
        // rather than reaching this guard — see the `has_delivery_thread`
        // checks in `spawn_subagent.rs` and `dispatch.rs::dispatch_subagent`.
        // Only a *direct* `spawn_async_subagent` call lands here.
        if parent_thread_id.is_none() {
            log::warn!(
                "[spawn_async_subagent] refusing fire-and-forget spawn with no delivery thread \
                 parent={} requested={} — directing caller to synchronous delegation (flow node / \
                 CLI / cron context, background result would be discarded)",
                parent.agent_definition_id,
                definition.id
            );
            return Ok(ToolResult::error(
                "spawn_async_subagent: no parent chat thread available to deliver the result \
                 into (this looks like a flow node, CLI, or cron run rather than an interactive \
                 chat turn). Fire-and-forget delegation has nowhere to land its result here and \
                 the sub-agent's work would be silently discarded. Use synchronous delegation \
                 instead: call `spawn_subagent` with `blocking: true`, or use a `delegate_*` \
                 tool — both run the sub-agent inline and hand you its output in this turn. \
                 For parallel work, model it as parallel flow nodes rather than background \
                 sub-agents.",
            ));
        }
        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace().cloned());
        let effective_action_root = workspace_descriptor
            .as_ref()
            .map(|workspace| {
                tracing::debug!(
                    workspace_root = %workspace.root.display(),
                    policy_id = %workspace.policy_id,
                    "[spawn_async_subagent] using ToolExecutionContext workspace root"
                );
                workspace.root.clone()
            })
            .or_else(|| {
                crate::openhuman::security::live_policy::current()
                    .map(|policy| policy.action_dir.clone())
            });
        let selector = SubagentSessionSelector {
            parent_session: parent_session.clone(),
            parent_thread_id: parent_thread_id.clone(),
            agent_id: definition.id.clone(),
            toolkit: toolkit_override.clone(),
            model: model_override.clone(),
            sandbox_mode: format!("{:?}", definition.sandbox_mode),
            action_root: subagent_sessions::action_root_key(effective_action_root.as_deref()),
            task_key: task_key.clone(),
        };

        let reusable = if force_fresh {
            match subagent_sessions::find_reusable(&store, &selector) {
                Ok(Some(session)) => {
                    let _ = running_subagents::cancel_by_session_in_workspace(
                        &session.subagent_session_id,
                        &parent_session,
                        &parent.workspace_dir,
                    );
                    if let Err(err) = subagent_sessions::close(&store, &session.subagent_session_id)
                    {
                        log::warn!(
                            "[subagent_reuse] fresh close failed parent_thread_id={} subagent_session_id={} agent_id={} task_key={} error={}",
                            parent_thread_id.as_deref().unwrap_or("none"),
                            session.subagent_session_id,
                            definition.id,
                            task_key,
                            err
                        );
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    log::warn!(
                        "[subagent_reuse] fresh lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        definition.id,
                        task_key,
                        err
                    );
                }
            }
            None
        } else {
            match subagent_sessions::find_reusable(&store, &selector) {
                Ok(session) => session,
                Err(err) => {
                    log::warn!(
                        "[subagent_reuse] lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        definition.id,
                        task_key,
                        err
                    );
                    None
                }
            }
        };
        let reuse_decision = subagent_sessions::reuse_decision(reusable.as_ref(), force_fresh);
        let follow_up_prompt = reusable_follow_up_message(&prompt, context.as_deref());

        if let Some(session) = reusable.as_ref() {
            if session.status == DurableSubagentStatus::Running {
                if let Some(ref running_task_id) = session.current_task_id {
                    match running_subagents::steer(
                        running_task_id,
                        &parent_session,
                        follow_up_prompt.clone(),
                        crate::openhuman::agent::harness::run_queue::QueueMode::Steer,
                    )
                    .await
                    {
                        Ok(()) => {
                            log::info!(
                                "[subagent_reuse] parent_thread_id={} subagent_session_id={} task_id={} agent_id={} reuse_decision={}",
                                parent_thread_id.as_deref().unwrap_or("none"),
                                session.subagent_session_id,
                                running_task_id,
                                definition.id,
                                reuse_decision.as_str()
                            );
                            let payload = async_subagent_ref_payload(
                                running_task_id,
                                &session.subagent_session_id,
                                &definition.id,
                                session.worker_thread_id.as_deref(),
                                true,
                                reuse_decision.as_str(),
                                "running",
                            );
                            return Ok(ToolResult::success(format!(
                                "Continued reusable async sub-agent `{}`. It is already running and will pick up the new instruction at its next step. \
                                 Use the structured reference below to send more input, wait, or perform a short timeout tick.\n\n[async_subagent_ref]\n{}\n[/async_subagent_ref]",
                                payload["agent_id"].as_str().unwrap_or("subagent"),
                                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
                            )));
                        }
                        Err(err) => {
                            log::warn!(
                                "[subagent_reuse] running steer failed parent_thread_id={} subagent_session_id={} task_id={} agent_id={} error={:?}",
                                parent_thread_id.as_deref().unwrap_or("none"),
                                session.subagent_session_id,
                                running_task_id,
                                definition.id,
                                err
                            );
                        }
                    }
                }
            }
        }

        let task_id = format!("sub-{}", uuid::Uuid::new_v4());
        let worker_thread_id = reusable
            .as_ref()
            .and_then(|session| session.worker_thread_id.clone())
            .or_else(|| {
                parent_thread_id.as_ref().and_then(|parent_thread_id| {
                    super::worker_thread::create_worker_thread(
                        parent.workspace_dir.clone(),
                        parent_thread_id,
                        &definition.id,
                        &task_title,
                        &prompt,
                    )
                    .ok()
                })
            });
        if let (Some(session), Some(worker_thread_id)) =
            (reusable.as_ref(), worker_thread_id.as_ref())
        {
            if session.worker_thread_id.as_deref() == Some(worker_thread_id.as_str()) {
                if let Err(err) = super::worker_thread::append_worker_user_message(
                    parent.workspace_dir.clone(),
                    worker_thread_id,
                    &definition.id,
                    &task_id,
                    &follow_up_prompt,
                ) {
                    log::warn!(
                        "[subagent_reuse] worker follow-up append failed parent_thread_id={} subagent_session_id={} worker_thread_id={} task_id={} error={}",
                        parent_thread_id.as_deref().unwrap_or("none"),
                        session.subagent_session_id,
                        worker_thread_id,
                        task_id,
                        err
                    );
                }
            }
        }
        let durable_session = match subagent_sessions::upsert_running(
            &store,
            SubagentSessionUpsert {
                selector,
                display_name: Some(definition.display_name().to_string()),
                task_title: task_title.clone(),
                worker_thread_id: worker_thread_id.clone(),
                task_id: task_id.clone(),
            },
            reusable.as_ref(),
        ) {
            Ok(session) => session,
            Err(err) => {
                log::warn!(
                    "[subagent_reuse] upsert failed parent_thread_id={} task_id={} agent_id={} reuse_decision={} error={}",
                    parent_thread_id.as_deref().unwrap_or("none"),
                    task_id,
                    definition.id,
                    reuse_decision.as_str(),
                    err
                );
                return Ok(ToolResult::error(format!(
                    "spawn_async_subagent: failed to persist reusable sub-agent session: {err}"
                )));
            }
        };

        let initial_history = reusable
            .as_ref()
            .and_then(|session| session.latest_history.clone())
            .map(|mut history| {
                history.push(ChatMessage::user(follow_up_prompt.clone()));
                history
            });

        log::info!(
            "[subagent_reuse] parent_thread_id={} subagent_session_id={} task_id={} agent_id={} reuse_decision={} task_key={}",
            parent_thread_id.as_deref().unwrap_or("none"),
            durable_session.subagent_session_id,
            task_id,
            definition.id,
            reuse_decision.as_str(),
            task_key
        );

        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            definition.id.clone(),
            "async".to_string(),
            task_id.clone(),
            prompt.chars().count(),
        );
        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: definition.id.clone(),
                    task_id: task_id.clone(),
                    mode: "async".to_string(),
                    dedicated_thread: worker_thread_id.is_some(),
                    prompt_chars: prompt.chars().count(),
                    prompt: prompt.clone(),
                    worker_thread_id: worker_thread_id.clone(),
                    display_name: Some(definition.display_name().to_string()),
                })
                .await;
        }

        // Steering channel + status channel so the parent can `steer_subagent`
        // this run mid-flight and `wait_subagent` for its result. The engine
        // drains `steer_queue` at iteration boundaries; `status_tx` publishes
        // the terminal state to any waiter.
        let steer_queue = RunQueue::new();
        let task_queue = steer_queue.clone();
        let (status_tx, status_rx) = running_subagents::status_channel();

        let background_parent = parent.clone();
        let background_workspace_dir = parent.workspace_dir.clone();
        let background_definition = definition.clone();
        let background_agent_id = definition.id.clone();
        let background_task_id = task_id.clone();
        let background_parent_session = parent_session.clone();
        let background_progress = progress_sink.clone();
        let background_worker_thread_id = worker_thread_id.clone();
        let background_store = store.clone();
        let background_subagent_session_id = durable_session.subagent_session_id.clone();
        let background_workspace_descriptor = workspace_descriptor.clone();
        let background_worktree_action_dir = background_workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        let background_thread_affinity_id = background_worker_thread_id
            .clone()
            .unwrap_or_else(|| background_subagent_session_id.clone());
        let background_initial_history = initial_history;
        // Capture the parent chat thread NOW (the spawning turn's thread) so the
        // finished result can be delivered back into it as a system turn.
        let background_parent_thread_id = parent_thread_id.clone();
        // Kept on this side (the closure moves its own clone) so the registry
        // entry knows which parent thread owns this sub-agent — that's how
        // `cancel_for_thread` aborts it when the thread is deleted.
        let register_parent_thread_id = background_parent_thread_id.clone();
        // Lifecycle-critical wiring: log the parent-thread binding so the
        // thread-close cancellation path (`cancel_for_thread`) is grep-friendly.
        log::debug!(
            "[spawn_async_subagent] register task_id={} parent_session={} parent_thread_id={}",
            task_id,
            parent_session,
            register_parent_thread_id.as_deref().unwrap_or("none")
        );
        let background_prompt = add_background_contract(&prompt);
        // The detached child starts on a fresh task, and a `tokio::task_local`
        // does not cross `tokio::spawn`. The parent's execution context and
        // chat thread are already re-installed inside the task for exactly that
        // reason; the turn's origin label and its workspace root are the other
        // two that have to travel, and they are captured **here**, on the
        // spawning task, rather than inside the closure where they would
        // already be gone. Without the origin every external-effect tool the
        // child calls reaches the approval gate unlabelled and is refused,
        // which is the whole of why a delegated coding task could not run a
        // shell.
        let join = tokio::spawn(crate::openhuman::agent::turn_origin::propagate(
            crate::openhuman::agent::turn_workspace::propagate(async move {
                let options = SubagentRunOptions {
                    skill_filter_override: None,
                    toolkit_override,
                    context,
                    model_override,
                    task_id: Some(background_task_id.clone()),
                    worker_thread_id: background_worker_thread_id.clone(),
                    initial_history: background_initial_history,
                    checkpoint_dir: None,
                    worktree_action_dir: background_worktree_action_dir,
                    workspace_descriptor: background_workspace_descriptor,
                    run_queue: Some(task_queue),
                };

                let result = with_parent_context(background_parent, async move {
                    crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
                        background_thread_affinity_id,
                        async move {
                            run_subagent(&background_definition, &background_prompt, options).await
                        },
                    )
                    .await
                })
                .await;

                match result {
                    Ok(outcome) => match outcome.status {
                        SubagentRunStatus::Completed => {
                            if let Err(err) = subagent_sessions::mark_finished(
                                &background_store,
                                &background_subagent_session_id,
                                &outcome.task_id,
                                &outcome.status,
                                outcome.final_history.clone(),
                            ) {
                                log::warn!(
                                "[subagent_reuse] mark_completed failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                            }
                            let _ = status_tx.send(SubagentStatus::Completed {
                                output: outcome.output.clone(),
                                iterations: outcome.iterations,
                            });
                            // A workflow proposal produced inside the child's tool
                            // history is durable state, not prose: persist it into
                            // the parent chat thread (survives reload / reconnect —
                            // the old socket-only delivery could silently drop it)
                            // and carry the full payload in the delivery notice so
                            // the follow-up turn can present it faithfully.
                            let delivery_summary = attach_workflow_proposal(
                                &background_workspace_dir,
                                background_parent_thread_id.as_deref(),
                                &outcome.task_id,
                                &outcome.agent_id,
                                &outcome.final_history,
                                outcome.output.clone(),
                            );
                            // Queue the finished result for idle-gated, batched
                            // delivery back into the parent chat (the session
                            // runtime drains this when the session is next idle).
                            crate::openhuman::agent::orchestration::background_completions::record_completion(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            delivery_summary,
                            background_parent_thread_id.clone(),
                        );
                            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                            if let Some(ref tx) = background_progress {
                                let _ = tx
                                    .send(AgentProgress::SubagentCompleted {
                                        agent_id: outcome.agent_id,
                                        task_id: outcome.task_id,
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
                        }
                        SubagentRunStatus::Incomplete { ref reason } => {
                            // Async sub-agent stopped short (stuck halt / iteration
                            // cap). Mark the session finished and deliver the PARTIAL
                            // progress back to the parent, framed so it is not
                            // mistaken for a completed result (#4096).
                            if let Err(err) = subagent_sessions::mark_finished(
                                &background_store,
                                &background_subagent_session_id,
                                &outcome.task_id,
                                &outcome.status,
                                outcome.final_history.clone(),
                            ) {
                                log::warn!(
                                "[subagent_reuse] mark_incomplete failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                            }
                            let framed = format!(
                                "[SUBAGENT_INCOMPLETE] the sub-agent {reason} and did not finish. \
                             Partial progress:\n{}",
                                outcome.output
                            );
                            let _ = status_tx.send(SubagentStatus::Completed {
                                output: framed.clone(),
                                iterations: outcome.iterations,
                            });
                            // An incomplete run may still have produced a full
                            // proposal before stalling — preserve it durably too.
                            let framed = attach_workflow_proposal(
                                &background_workspace_dir,
                                background_parent_thread_id.as_deref(),
                                &outcome.task_id,
                                &outcome.agent_id,
                                &outcome.final_history,
                                framed,
                            );
                            crate::openhuman::agent::orchestration::background_completions::record_completion(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            framed,
                            background_parent_thread_id.clone(),
                        );
                            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                            if let Some(ref tx) = background_progress {
                                let _ = tx
                                    .send(AgentProgress::SubagentCompleted {
                                        agent_id: outcome.agent_id,
                                        task_id: outcome.task_id,
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
                        }
                        SubagentRunStatus::AwaitingUser { ref question, .. } => {
                            if let Err(err) = subagent_sessions::mark_finished(
                                &background_store,
                                &background_subagent_session_id,
                                &outcome.task_id,
                                &outcome.status,
                                outcome.final_history.clone(),
                            ) {
                                log::warn!(
                                "[subagent_reuse] mark_awaiting_user failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                outcome.task_id,
                                outcome.agent_id,
                                err
                            );
                            }
                            let _ = status_tx.send(SubagentStatus::AwaitingUser {
                                question: question.clone(),
                            });
                            let error = format!(
                            "async sub-agent requested user clarification and was not continued: {question}"
                        );
                            // #4896: a detached child that pauses for input won't
                            // continue on its own — queue a framed notice so the
                            // parent chat learns the delegated task needs input,
                            // instead of finalizing silently on "Accepted". Rides the
                            // same idle-gated background_delivery path as a success.
                            crate::openhuman::agent::orchestration::background_completions::record_awaiting_input(
                            background_parent_session.clone(),
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            question,
                            background_parent_thread_id.clone(),
                        );
                            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            error.clone(),
                        );
                            if let Some(ref tx) = background_progress {
                                let _ = tx
                                    .send(AgentProgress::SubagentFailed {
                                        agent_id: outcome.agent_id,
                                        task_id: outcome.task_id,
                                        error,
                                    })
                                    .await;
                            }
                        }
                    },
                    Err(err) => {
                        let error = err.to_string();
                        if let Err(store_err) = subagent_sessions::mark_failed(
                            &background_store,
                            &background_subagent_session_id,
                            &background_task_id,
                            error.clone(),
                        ) {
                            log::warn!(
                            "[subagent_reuse] mark_failed failed subagent_session_id={} task_id={} agent_id={} error={}",
                            background_subagent_session_id,
                            background_task_id,
                            background_agent_id,
                            store_err
                        );
                        }
                        let _ = status_tx.send(SubagentStatus::Failed {
                            error: error.clone(),
                        });
                        // #4896: a detached child that errors previously only
                        // published an event — nothing reached chat, so the parent
                        // turn finalized on "Accepted" and the failure was lost.
                        // Queue a framed failure notice so background_delivery
                        // surfaces it as a follow-up chat turn.
                        crate::openhuman::agent::orchestration::background_completions::record_failure(
                        background_parent_session.clone(),
                        background_task_id.clone(),
                        background_agent_id.clone(),
                        &error,
                        background_parent_thread_id.clone(),
                    );
                        crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                        background_parent_session,
                        background_task_id.clone(),
                        background_agent_id.clone(),
                        error.clone(),
                    );
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentFailed {
                                    agent_id: background_agent_id,
                                    task_id: background_task_id,
                                    error,
                                })
                                .await;
                        }
                    }
                }
            }),
        ));

        // Register *after* spawn so the AbortHandle is available. The task owns
        // `status_tx`; this side holds `status_rx` for `wait_subagent`.
        running_subagents::register(
            task_id.clone(),
            definition.id.clone(),
            parent_session.clone(),
            parent.session_parent_prefix.clone(),
            Some(durable_session.subagent_session_id.clone()),
            parent.workspace_dir.clone(),
            register_parent_thread_id,
            steer_queue,
            join.abort_handle(),
            status_rx,
        );

        let payload = async_subagent_ref_payload(
            &task_id,
            &durable_session.subagent_session_id,
            &definition.id,
            worker_thread_id.as_deref(),
            reusable.is_some(),
            reuse_decision.as_str(),
            "running",
        );
        let payload_json = match serde_json::to_string(&payload) {
            Ok(serialized) => {
                log::debug!(
                    "[spawn_async_subagent] serialized async reference payload bytes={}",
                    serialized.len()
                );
                serialized
            }
            Err(error) => {
                log::debug!(
                    "[spawn_async_subagent] failed to serialize async reference payload: {}",
                    error
                );
                "{}".to_string()
            }
        };
        log::debug!("[spawn_async_subagent] formatting accepted response");
        Ok(ToolResult::success(format_async_subagent_accepted(
            payload["agent_id"].as_str().unwrap_or("subagent"),
            &payload_json,
        )))
    }
}
