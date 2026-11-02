type DetachedSpawnParent<'a> = &'a RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>;

impl SpawnSubagentTool {
    async fn execute_with_context_inner(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        detached_parent: Option<DetachedSpawnParent<'_>>,
    ) -> anyhow::Result<ToolResult> {
        spawn_subagent_preflight!(
            dispatch,
            agent_id,
            prompt,
            context,
            model_override,
            task_title,
            task_key,
            force_fresh,
            run_context,
            args,
            detached_parent
        );

        let parent = match run_context.parent.clone() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "spawn_subagent called outside of an agent turn",
                ));
            }
        };

        // Refuse out-of-scope callers before any session or filesystem effects.
        if let Err(error) = running_subagents::admit_spawn(&dispatch, &parent.workspace_dir) {
            return Ok(ToolResult::error(format!(
                "spawn_subagent: registration refused: {error}"
            )));
        }

        spawn_subagent_definition!(definition, run_context, agent_id, parent);
        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();
        let parent_thread_id = tool_context
            .and_then(ToolRunContext::thread_id)
            .or(run_context.thread_id.as_deref())
            .map(str::to_owned);
        let store =
            match SubagentSessionStore::new_in_context(&dispatch, parent.workspace_dir.clone()) {
                Ok(store) => store,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent: failed to open the caller-scoped session store: {error}"
                    )));
                }
            };
        let jobs = crate::core::runtime::slot_for::<
            tinyagents_orchestration::subagent::SubAgentJobRegistry,
        >(&dispatch);
        let driver_store = match subagent_sessions::open_for_context(
            &dispatch,
            parent.workspace_dir.clone(),
            parent_session.clone(),
            parent_thread_id.clone(),
            Some((*jobs).clone()),
        ) {
            Ok(store) => store,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: failed to open the caller-scoped driver session store: {error}"
                )));
            }
        };
        let workspace_descriptor = tool_context
            .and_then(|ctx| ctx.workspace().cloned())
            .or_else(|| run_context.workspace.clone());
        let task_workspace = workspace_descriptor
            .clone()
            .unwrap_or_else(|| tinytools::WorkspaceDescriptor::new(parent.workspace_dir.clone()));
        let resolved_model = definition.model.resolve(parent.model_name.as_str());
        let target_model = model_override.as_deref().unwrap_or(&resolved_model);
        let model_supports_vision = run_context.hosted_base.as_ref().is_some_and(|base| {
            crate::inference::model_context::model_supports_vision(target_model, &base.config)
        });
        if crate::attachments::should_forward_parent_images(&prompt)
            && !run_context.attachment_placeholders.is_empty()
            && (definition.id == "vision_agent" || model_supports_vision)
        {
            prompt = format!(
                "{}\n\n{}",
                run_context.attachment_placeholders.join("\n"),
                prompt
            );
        }
        prompt = match crate::attachments::delegation_prompt(
            &dispatch,
            &prompt,
            &args,
            workspace_descriptor.as_ref(),
            run_context.origin.as_ref(),
        )
        .await
        {
            Ok(prompt) => prompt,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "image forwarding failed: {error}"
                )));
            }
        };
        if definition.id == "vision_agent" {
            match crate::attachments::has_resolvable_image(
                &dispatch,
                &prompt,
                workspace_descriptor.as_ref(),
                run_context.origin.as_ref(),
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    return Ok(ToolResult::error(
                        "vision_agent requires a resolvable image attachment or image_paths.",
                    ));
                }
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "vision image unavailable: {error}"
                    )));
                }
            }
        }
        let effective_action_root = workspace_descriptor
            .as_ref()
            .map(|workspace| {
                tracing::debug!(
                    workspace_root = %workspace.root.display(),
                    policy_id = %workspace.policy_id,
                    "[spawn_subagent] using ToolExecutionContext workspace root"
                );
                workspace.root.clone()
            })
            .or_else(|| {
                crate::security::live_policy::effective_in(&dispatch)
                    .map(|policy| policy.action_dir.clone())
            });
        let selector = SubagentSessionSelector {
            parent_session: parent_session.clone(),
            parent_thread_id: parent_thread_id.clone(),
            agent_id: definition.id.clone(),
            model: model_override.clone(),
            sandbox_mode: format!("{:?}", definition.sandbox_mode),
            action_root: subagent_sessions::action_root_key(effective_action_root.as_deref()),
            task_key: task_key.clone(),
        };

        let reusable = match reusable_driver_session_for_spawn(
            &driver_store,
            &store,
            &dispatch,
            &parent.workspace_dir,
            &selector,
            force_fresh,
        )
        .await
        {
            Ok(reusable) => reusable,
            Err(error) => return Ok(ToolResult::error(format!("spawn_subagent: {error}"))),
        };
        let reuse_decision = subagent_sessions::reuse_decision(reusable.as_ref(), force_fresh);
        let follow_up_prompt = reusable_follow_up_message(&prompt, context.as_deref());

        if let Some(session) = reusable
            .as_ref()
            .filter(|session| session.status == DurableSubagentStatus::AwaitingUser)
        {
            let Some(task_id) = session.current_task_id.as_deref() else {
                return Ok(ToolResult::error(
                    "spawn_subagent: paused reusable session has no current task id",
                ));
            };
            let checkpoint_dir = parent.workspace_dir.join(".openhuman/subagent_checkpoints");
            let original_key = match subagent_sessions::resolve_driver_lifecycle_key(
                driver_store.as_ref(),
                &session.subagent_session_id,
                &checkpoint_dir,
            )
            .await
            {
                Ok(key) => key,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent: reusable session has no valid lifecycle key: {error}"
                    )));
                }
            };
            let checkpoint = match crate::agent::subagent_host::load_subagent_checkpoint(
                &checkpoint_dir,
                task_id,
            ) {
                Ok(checkpoint) => checkpoint,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent: failed to load paused checkpoint for task '{task_id}': {error}"
                    )));
                }
            };
            if checkpoint.agent_id != definition.id
                || checkpoint.task_id != task_id
                || checkpoint.task_key.as_ref() != Some(&original_key)
            {
                return Ok(ToolResult::error(
                    "spawn_subagent: paused checkpoint does not match the reusable session",
                ));
            }
            let checkpoint_resume = crate::agent::subagent_host::checkpoint_resume(&checkpoint);
            let completion_target =
                match crate::agent::orchestration::background_completions::CompletionTarget::new(
                    dispatch.clone(),
                    parent.workspace_dir.clone(),
                    parent_session.clone(),
                    parent_thread_id.clone(),
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        return Ok(ToolResult::error(format!(
                            "spawn_subagent: parent lifecycle admission failed: {error}"
                        )));
                    }
                };
            if let Some(thread_id) = parent_thread_id.as_deref() {
                if crate::agent::orchestration::background_completions::mark_stopped_task_if_thread_stopped(
                    &dispatch,
                    &parent.workspace_dir,
                    thread_id,
                    task_id,
                ) {
                    return Ok(ToolResult::error(
                        "spawn_subagent: parent thread is stopped or deleted",
                    ));
                }
            }
            let resumed_session = match upsert_driver_session(
                &driver_store,
                &dispatch,
                &parent.workspace_dir,
                SubagentSessionUpsert {
                    selector: selector.clone(),
                    display_name: Some(definition.display_name().to_string()),
                    task_title: task_title.clone(),
                    worker_thread_id: session.worker_thread_id.clone(),
                    task_id: task_id.to_owned(),
                    lifecycle_key: Some(original_key.clone()),
                    parent_epoch: completion_target.parent_epoch(),
                },
                false,
            )
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent: failed to resume paused session: {error}"
                    )));
                }
            };

            crate::agent::orchestration::subagent_events::publish_subagent_spawned(
                parent_session.clone(),
                definition.id.clone(),
                "async".to_string(),
                task_id.to_owned(),
                follow_up_prompt.chars().count(),
            );
            completion::report_spawned_progress(
                progress_sink.clone(),
                definition.id.clone(),
                task_id.to_owned(),
                follow_up_prompt.clone(),
                session.worker_thread_id.clone(),
                definition.display_name().to_string(),
                crate::tools::host_extensions::tool_call_id(tool_context),
            )
            .await;

            let completion = completion::make_completion(
                dispatch.clone().into_core(),
                detached_parent.data.origin.clone(),
                progress_sink.clone(),
                parent.workspace_dir.clone(),
                dispatch.clone(),
                definition.id.clone(),
                task_id.to_owned(),
                task_key.clone(),
                parent_session.clone(),
                completion_target.clone(),
                session.worker_thread_id.clone(),
                driver_store.clone(),
                resumed_session.subagent_session_id.clone(),
                parent_thread_id.clone(),
                0,
            );
            let options = SubagentRunOptions {
                skill_filter_override: checkpoint.skill_filter_override,
                context: context.clone(),
                model_override: checkpoint.model_override,
                task_id: Some(task_id.to_owned()),
                thread_id: Some(
                    session
                        .worker_thread_id
                        .clone()
                        .unwrap_or_else(|| resumed_session.subagent_session_id.clone()),
                ),
                run_context: detached_parent.data.child(),
                worker_thread_id: session.worker_thread_id.clone(),
                initial_history: None,
                checkpoint_dir: Some(checkpoint_dir),
                workspace_descriptor: workspace_descriptor.clone(),
                run_queue: None,
                message_run_queue: None,
            };
            if let Err(error) = crate::agent::orchestration::completion_owners::note_driver_task(
                &dispatch,
                task_id,
                parent.workspace_dir.clone(),
                task_workspace.clone(),
                parent_session.clone(),
                parent_thread_id.clone(),
            ) {
                let message = format!("driver owner registration refused: {error}");
                let _ = driver_store
                    .mark_failed(
                        &resumed_session.subagent_session_id,
                        task_id,
                        message.clone(),
                    )
                    .await;
                completion_target
                    .failed(task_id, &definition.id, &message)
                    .await;
                crate::agent::orchestration::completion_owners::finish_driver_task(
                    &dispatch, task_id,
                );
                return Ok(ToolResult::error(format!("spawn_subagent: {message}")));
            }
            if let Err(error) = crate::agent::subagent_host::resume_async_via_driver_tool(
                detached_parent,
                original_key,
                checkpoint_resume,
                definition.clone(),
                follow_up_prompt,
                options,
                completion,
            )
            .await
            {
                let message = error.to_string();
                let _ = driver_store
                    .mark_failed(
                        &resumed_session.subagent_session_id,
                        task_id,
                        message.clone(),
                    )
                    .await;
                completion_target
                    .failed(task_id, &definition.id, &message)
                    .await;
                crate::agent::orchestration::completion_owners::finish_driver_task(
                    &dispatch, task_id,
                );
                crate::agent::orchestration::subagent_events::publish_subagent_failed(
                    parent_session,
                    task_id.to_owned(),
                    definition.id.clone(),
                    message.clone(),
                );
                if let Some(progress) = progress_sink {
                    let _ = progress
                        .send(AgentProgress::SubagentFailed {
                            agent_id: definition.id,
                            task_id: task_id.to_owned(),
                            error: message,
                        })
                        .await;
                }
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: failed to resume detached sub-agent: {error}"
                )));
            }
            return Ok(accepted_spawn_result(
                task_id,
                &resumed_session.subagent_session_id,
                &definition.id,
                session.worker_thread_id.as_deref(),
                true,
                reuse_decision.as_str(),
                &task_key,
            ));
        }

        if let Some(session) = reusable.as_ref() {
            if session.status == DurableSubagentStatus::Running {
                if let Some(ref running_task_id) = session.current_task_id {
                    match steer_reusable_session(
                        detached_parent,
                        &session.subagent_session_id,
                        follow_up_prompt.clone(),
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
                                &session.task_key,
                                &definition.id,
                                session.worker_thread_id.as_deref(),
                                true,
                                reuse_decision.as_str(),
                                "running",
                            );
                            return Ok(ToolResult::success(format!(
                                "Continued reusable async sub-agent `{}`. It is already running and will pick up the new instruction at its next step. \
                                 Its result is delivered automatically on a later turn.\n\n[async_subagent_ref]\n{}\n[/async_subagent_ref]",
                                payload["agent_id"].as_str().unwrap_or("subagent"),
                                serde_json::to_string(&payload)
                                    .unwrap_or_else(|_| "{}".to_string())
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
                            return Ok(ToolResult::error(format!(
                                "spawn_subagent: refusing to start a duplicate child for reusable task '{running_task_id}': {err}"
                            )));
                        }
                    }
                }
            }
        }

        let task_id = format!("sub-{}", uuid::Uuid::new_v4());
        if let Some(thread_id) = parent_thread_id.as_deref() {
            if crate::agent::orchestration::background_completions::mark_stopped_task_if_thread_stopped(
                &dispatch,
                &parent.workspace_dir,
                thread_id,
                &task_id,
            ) {
                return Ok(ToolResult::error(
                    "spawn_subagent: parent thread is stopped or deleted",
                ));
            }
        }
        // Capture the admitted generation before creating the worker/session.
        let completion_target =
            match crate::agent::orchestration::background_completions::CompletionTarget::new(
                dispatch.clone(),
                parent.workspace_dir.clone(),
                parent_session.clone(),
                parent_thread_id.clone(),
            ) {
                Ok(target) => target,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent: parent lifecycle admission failed: {error}"
                    )));
                }
            };
        let worker_thread_id = reusable
            .as_ref()
            .and_then(|session| session.worker_thread_id.clone())
            .or_else(|| {
                parent_thread_id.as_ref().and_then(|parent_thread_id| {
                    crate::agent::subagent_host::create_worker_thread(
                        &dispatch,
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
                crate::agent::subagent_host::append_reuse_follow_up(
                    &dispatch,
                    parent.workspace_dir.clone(),
                    worker_thread_id,
                    &definition.id,
                    &task_id,
                    &follow_up_prompt,
                    (parent_thread_id.as_deref(), &session.subagent_session_id),
                );
            }
        }
        let durable_session = match upsert_driver_session(
            &driver_store,
            &dispatch,
            &parent.workspace_dir,
            SubagentSessionUpsert {
                selector,
                display_name: Some(definition.display_name().to_string()),
                task_title: task_title.clone(),
                worker_thread_id: worker_thread_id.clone(),
                task_id: task_id.clone(),
                lifecycle_key: Some(
                    tinyagents_orchestration::subagent::SubagentTaskKey::from_context(
                        detached_parent,
                        task_id.clone(),
                        parent_thread_id.clone(),
                    ),
                ),
                parent_epoch: completion_target.parent_epoch(),
            },
            force_fresh,
        )
        .await
        {
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
                    "spawn_subagent: failed to persist reusable sub-agent session: {err}"
                )));
            }
        };

        let (initial_history, seed_len) =
            reused_session_history(reusable.as_ref(), &follow_up_prompt);

        log::info!(
            "[subagent_reuse] parent_thread_id={} subagent_session_id={} task_id={} agent_id={} reuse_decision={} task_key={}",
            parent_thread_id.as_deref().unwrap_or("none"),
            durable_session.subagent_session_id,
            task_id,
            definition.id,
            reuse_decision.as_str(),
            task_key
        );

        if let Some(thread_id) = parent_thread_id.as_deref() {
            crate::agent::orchestration::background_completions::note_thread_workspace(
                &dispatch,
                thread_id,
                &parent.workspace_dir,
            );
            crate::agent::orchestration::background_completions::note_session_thread(
                &dispatch,
                &parent_session,
                thread_id,
            );
        }
        if let Err(error) = crate::agent::orchestration::completion_owners::note_driver_task(
            &dispatch,
            &task_id,
            parent.workspace_dir.clone(),
            task_workspace.clone(),
            parent_session.clone(),
            parent_thread_id.clone(),
        ) {
            if let Err(store_error) = driver_store
                .mark_failed(
                    &durable_session.subagent_session_id,
                    &task_id,
                    error.clone(),
                )
                .await
            {
                log::warn!(
                    "[subagent_reuse] mark_failed after owner admission refusal failed subagent_session_id={} task_id={} error={store_error}",
                    durable_session.subagent_session_id,
                    task_id
                );
            }
            return Ok(ToolResult::error(format!(
                "spawn_subagent: registration refused: {error}"
            )));
        }

        crate::agent::orchestration::subagent_events::publish_subagent_spawned(
            parent_session.clone(),
            definition.id.clone(),
            "async".to_string(),
            task_id.clone(),
            prompt.chars().count(),
        );
        completion::report_spawned_progress(
            progress_sink.clone(),
            definition.id.clone(),
            task_id.clone(),
            prompt.clone(),
            worker_thread_id.clone(),
            definition.display_name().to_string(),
            crate::tools::host_extensions::tool_call_id(tool_context),
        )
        .await;

        let background_workspace_dir = parent.workspace_dir.clone();
        let background_dispatch = dispatch.clone();
        let background_agent_id = definition.id.clone();
        let background_task_id = task_id.clone();
        let background_task_key = task_key.clone();
        let background_parent_session = parent_session.clone();
        let background_worker_thread_id = worker_thread_id.clone();
        let background_store = driver_store.clone();
        let background_subagent_session_id = durable_session.subagent_session_id.clone();
        let background_thread_id = background_worker_thread_id
            .clone()
            .unwrap_or_else(|| background_subagent_session_id.clone());
        let background_parent_thread_id = parent_thread_id.clone();
        log::debug!(
            "[spawn_subagent] register task_id={} parent_session={} parent_thread_id={}",
            task_id,
            parent_session,
            background_parent_thread_id.as_deref().unwrap_or("none")
        );
        let completion_core = dispatch.clone().into_core();
        let completion_origin = detached_parent.data.origin.clone();
        let completion_worker_thread_id = background_worker_thread_id.clone();
        let registration_progress = progress_sink.clone();
        let registration_completion_target = completion_target.clone();
        let registration_parent_session = parent_session.clone();
        let registration_agent_id = definition.id.clone();
        let registration_task_id = task_id.clone();
        let completion = completion::make_completion(
            completion_core,
            completion_origin,
            progress_sink.clone(),
            background_workspace_dir,
            background_dispatch,
            background_agent_id,
            background_task_id,
            background_task_key,
            background_parent_session,
            completion_target,
            completion_worker_thread_id,
            background_store,
            background_subagent_session_id,
            background_parent_thread_id,
            seed_len,
        );
        let background_prompt = add_background_contract(&prompt);
        let options = SubagentRunOptions {
            skill_filter_override: None,
            context,
            model_override,
            task_id: Some(task_id.clone()),
            thread_id: Some(background_thread_id),
            run_context: detached_parent.data.child(),
            worker_thread_id: background_worker_thread_id,
            initial_history,
            checkpoint_dir: None,
            workspace_descriptor: Some(task_workspace),
            run_queue: None,
            message_run_queue: None,
        };
        if let Err(error) = crate::agent::subagent_host::spawn_async_via_driver_tool(
            detached_parent,
            definition.clone(),
            background_prompt,
            options,
            completion,
        )
        .await
        {
            if let Err(store_err) = driver_store
                .mark_failed(
                    &durable_session.subagent_session_id,
                    &task_id,
                    error.to_string(),
                )
                .await
            {
                log::warn!(
                    "[subagent_reuse] mark_failed after driver registration error subagent_session_id={} task_id={} agent_id={} error={}",
                    durable_session.subagent_session_id,
                    task_id,
                    definition.id,
                    store_err
                );
            }
            let message = error.to_string();
            registration_completion_target
                .failed(&registration_task_id, &registration_agent_id, &message)
                .await;
            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                registration_parent_session,
                registration_task_id.clone(),
                registration_agent_id.clone(),
                message.clone(),
            );
            crate::agent::orchestration::completion_owners::finish_driver_task(
                &dispatch,
                &registration_task_id,
            );
            if let Some(progress) = registration_progress {
                let _ = progress
                    .send(AgentProgress::SubagentFailed {
                        agent_id: registration_agent_id,
                        task_id: registration_task_id,
                        error: message,
                    })
                    .await;
            }
            return Ok(ToolResult::error(format!(
                "spawn_subagent: failed to register detached sub-agent: {error}"
            )));
        }

        Ok(accepted_spawn_result(
            &task_id,
            &durable_session.subagent_session_id,
            &definition.id,
            worker_thread_id.as_deref(),
            reusable.is_some(),
            reuse_decision.as_str(),
            &task_key,
        ))
    }
}
