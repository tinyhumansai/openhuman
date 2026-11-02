use super::*;

impl OpenHumanSessionHost {
    pub(in crate::agent::session_host) fn ensure_runtime_session(&mut self) -> Result<()> {
        if self.runtime_session.is_some() {
            return Ok(());
        }
        let context_config = &self.context_config;
        let tool_result_budget_bytes = context_config.tool_result_budget_bytes;
        let tokenjuice_compaction_enabled = context_config.compaction_enabled;
        let microcompact_keep_recent = if context_config.microcompact_enabled {
            context_config.microcompact_keep_recent
        } else {
            0
        };
        let autocompact_enabled = context_config.enabled && context_config.autocompact_enabled;
        let compaction = context_config.compaction_settings();
        let artifact_store = crate::agent::harness::tool_result_artifacts::build_artifact_store(
            &self.workspace_dir,
            &self.event_session_id,
        );

        // `[components] juice = false`: truncation only, no compaction/summary.
        let juice_on =
            (self.runtime_config.as_ref()).is_none_or(|c| crate::inference::tokenjuice::enabled(c));
        let context_mw = TurnContextMiddleware {
            tool_result_budget_bytes,
            payload_summarizer: self.payload_summarizer.clone().filter(|_| juice_on),
            // Rooted outside the project; see `tool_result_artifacts` (#6408, #6483).
            artifact_store,
            tokenjuice_compaction_enabled: tokenjuice_compaction_enabled && juice_on,
            tokenjuice_compression: self.tokenjuice_compression,
            runtime_config: self.runtime_config.clone(),
            microcompact_keep_recent,
            autocompact_enabled,
            compaction,
            transcript_snapshot: None,
        };
        let driver = Arc::new(OpenHumanSessionDriver::new(
            self.turn_model_source.clone(),
            self.tool_dispatcher.clone(),
            self.model_name.clone(),
            self.temperature,
            self.config.max_tool_iterations,
            self.model_vision,
            self.run_queue.clone(),
            self.workspace_descriptor.clone(),
            self.resolved_definition()
                .map(|definition| definition.sandbox_mode)
                .unwrap_or(tinyagents_definition::SandboxMode::None),
            self.hosted_base.clone(),
            self.agent_definition_id.clone(),
        ));
        // Thread-bound roots resume their deterministic transcript; other
        // turns keep the per-run stem path. Reuse one locator for the initial
        // binding and each resume because the runtime compares locator identity.
        let injected_locator = self.session_history_locator.clone();
        let resume_target = injected_locator
            .as_ref()
            .map(|locator| match self.session.clone() {
                Some(session) => TranscriptTarget::for_session(
                    Arc::clone(locator),
                    session,
                    self.runtime_transcript_meta(),
                ),
                None => TranscriptTarget::new(
                    Arc::clone(locator),
                    self.injected_transcript_stem(),
                    self.runtime_transcript_meta(),
                )
                .with_resume_agent(self.agent_definition_name.clone()),
            });
        {
            let mut state = self
                .runtime_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.context_middleware = Some(context_mw.clone());
            state.required_output = self
                .config
                .required_output
                .as_ref()
                .map(crate::agent::tinyagents::config::required_output_from);
            state.prelude = Some(self.create_turn_prelude(false));
        }
        let hooks = Arc::new(OpenHumanSessionHooks::new(
            {
                let resume_target = resume_target.clone();
                let state = self.runtime_state.clone();
                move |_, options, _| {
                    let resume_target = resume_target.clone();
                    begin_turn_resume(
                        &mut state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()),
                        &mut options.resume,
                    );
                    Box::pin(async move {
                        Ok(ResumePreparation {
                            transcript: resume_target,
                        })
                    })
                }
            },
            {
                let state = self.runtime_state.clone();
                move |request, options, view| {
                    let state = state.clone();
                    let request_base_len = view.history.len()
                        + usize::from(view.history.last() != Some(&request.input));
                    Box::pin(async move {
                        let transcript_snapshot =
                            crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink::default();
                        transcript_snapshot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .request_base_len = request_base_len;
                        let (prelude, pending_post_commit) = {
                            let mut state = state
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            state.active_terminal_sidecar =
                                Some(options.run_context.data.session_sidecar.clone());
                            (state.prelude.clone(), state.pending_post_commit.take())
                        };
                        let prelude = prelude.ok_or({
                            tinyagents_runtime::RuntimeError::MissingDependency(
                                "OpenHumanTurnPrelude",
                            )
                        })?;
                        // The previous turn's goal accounting was deferred past
                        // its completion; settle it before this turn reads the goal.
                        post_commit::await_pending(pending_post_commit).await;
                        let new_session = !view.resumed && view.history.is_empty();
                        prelude
                            .refresh_turn_boundary(new_session)
                            .await
                            .map_err(|error| {
                                tinyagents_runtime::RuntimeError::Driver(error.to_string())
                            })?;
                        let context_window = prelude
                            .turn_model_source
                            .effective_context_window(&prelude.model_name)
                            .await
                            .unwrap_or_default();
                        options
                            .run_context
                            .data
                            .session_sidecar
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .context_window = context_window;
                        let original_input = request.input.clone();
                        let original_user_message = user_text_with_markers(&original_input);
                        prelude.begin_user_effects(request, &options.run_context.data);
                        let turn = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take_turn_inputs();
                        let origin = options.run_context.data.origin.clone();
                        let (enriched, memory_turn) = futures::join!(
                            prelude.enrich_request(
                                &original_user_message,
                                &turn,
                                &mut options.run_context.data,
                            ),
                            // Boxed: the hook's future (config load, engine
                            // calls) would otherwise be inlined into this
                            // already-large hook future.
                            Box::pin(prelude.memory_pre_turn(
                                view.history,
                                view.committed_turns,
                                view.history.last().filter(|last| **last == request.input),
                                origin,
                            )),
                        );
                        options.run_context.data.memory_turn = memory_turn;
                        // Keep trusted typed media (channel attachments, voice
                        // notes) on the input; text-only turns stay strings.
                        request.input = match enrichment::enriched_typed_user_input(
                            &original_input,
                            &original_user_message,
                            &enriched,
                        ) {
                            Ok(Some(input)) => input,
                            Ok(None) => user_message_from_text(&enriched),
                            Err(()) => {
                                return Err(tinyagents_runtime::RuntimeError::Driver(
                                    "host enrichment could not preserve typed user media".into(),
                                ));
                            }
                        };
                        let mut preparation = prelude
                            .prepare_turn(new_session, &view, &transcript_snapshot, request)
                            .await?;
                        if turn.overrides.suppress_tools {
                            // One-off tool-less turn: must not become the
                            // thread's recorded tool list.
                            preparation.tools = Some(ToolSnapshot::default().exact());
                        }
                        let (
                            mut current_tools,
                            mut current_synthesized_tools,
                            policy_session,
                            policy_channel,
                        ) = prelude.current_tool_source();
                        if turn.overrides.suppress_tools {
                            current_tools = Arc::new(Vec::new());
                            current_synthesized_tools = Arc::new(Vec::new());
                        }
                        let mut middleware = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .context_middleware
                            .clone()
                            .ok_or({
                                tinyagents_runtime::RuntimeError::MissingDependency(
                                    "TurnContextMiddleware",
                                )
                            })?;
                        middleware.transcript_snapshot = Some(transcript_snapshot);
                        options.run_context.data.context_middleware = Some(middleware);
                        options.run_context.data.current_tools = Some(current_tools);
                        if !turn.overrides.suppress_tools {
                            options.run_context.data.deferred_tool_names =
                                Arc::new(prelude.current_deferred_tool_names());
                        }
                        options.run_context.data.current_synthesized_tools =
                            Some(current_synthesized_tools);
                        options.run_context.data.tool_rules =
                            prelude.turn_tool_rules(&policy_channel, &options.run_context.data);
                        options.run_context.data.tool_policy =
                            Some(crate::agent::tinyagents::ToolPolicyEnforcement {
                                policy: prelude.tool_policy.clone(),
                                session: policy_session,
                                channel: policy_channel,
                                agent_definition_id: prelude.agent_definition_id.clone(),
                            });
                        options.run_context.data.required_output = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .required_output
                            .clone();
                        Ok(preparation)
                    })
                }
            },
            |outcome, _| {
                Box::pin(async move {
                    if outcome
                        .output
                        .as_deref()
                        .is_none_or(|output| output.trim().is_empty())
                    {
                        return Err(tinyagents_runtime::RuntimeError::Driver(
                            "driver returned an empty final response after repair".into(),
                        ));
                    }
                    Ok(())
                })
            },
            |_| Box::pin(async { Ok(()) }),
            {
                let state = self.runtime_state.clone();
                move |terminal| {
                    let state = state.clone();
                    Box::pin(async move {
                        let sidecar = post_commit::take_terminal_sidecar(&state, terminal.clone());
                        if let Some(sidecar) = sidecar {
                            post_commit::finalize_deferred_terminal(&sidecar, &terminal).await;
                        }
                        Ok(())
                    })
                }
            },
        ));
        // Bind the hook's native target early so both thread and latest-agent
        // resumes can read recorded declarations before host admission.
        let mut builder = SessionBuilder::new(driver)
            .codec(Arc::new(OpenHumanTranscriptCodec))
            .hooks(hooks)
            .post_commit_sink(Arc::new(post_commit::OpenHumanPostCommitSink {
                state: self.runtime_state.clone(),
                hooks: self.post_turn_hooks.clone(),
                identity: post_commit::TurnIdentityOwned {
                    session_id: self.event_session_id.clone(),
                    agent_id: self.agent_definition_id.clone(),
                    channel: self.event_channel.clone(),
                },
            }))
            .retain_recorded_tools(!self.host_only && !self.host_env.fresh_tool_snapshot);
        let meta = self.runtime_transcript_meta();
        builder = if let Some(locator) = injected_locator {
            match self.session.clone() {
                Some(session) => builder.session(locator, session, meta),
                None => builder
                    .transcript(locator, self.injected_transcript_stem(), meta)
                    .resume_agent(self.agent_definition_name.clone()),
            }
        } else if self.thread_id.is_some() && self.session_parent_prefix.is_none() {
            builder.identity_resolver(
                self.runtime_session_key(),
                meta,
                Arc::new(self.session_identity_resolver()),
            )
        } else {
            builder
                .transcript(
                    self.subagent_transcript_locator(),
                    self.injected_transcript_stem(),
                    meta,
                )
                .resume_agent(self.agent_definition_name.clone())
        };
        self.runtime_session = Some(
            builder
                .build()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        );
        Ok(())
    }
}
