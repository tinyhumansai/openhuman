use super::*;

#[async_trait]
impl SessionDriver<OpenHumanRunContext> for OpenHumanSessionDriver {
    async fn execute(
        &self,
        mut request: DriverRequest<OpenHumanRunContext>,
    ) -> Result<DriverOutcome, DriverFailure> {
        // Use the policy and context prepared for this exact request.
        let tool_policy = request
            .run_context
            .data
            .tool_policy
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing its request-scoped tool policy")
            })?;
        let mut context_mw = request
            .run_context
            .data
            .context_middleware
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing its request-scoped context"))?;
        let hosted_base = self.hosted_base.clone().ok_or_else(|| {
            driver_failure(
                "hosted root invocation is unavailable because the session has no hosted authority",
            )
        })?;
        let security = crate::agent::tinyagents::host::OpenHumanSecurityGate::new(
            hosted_base.security_policy.clone(),
            Vec::new(),
        )
        .with_input_screening(!request.run_context.data.untrusted_input_turn)
        .with_approval(request.run_context.data.approval.clone())
        .with_dispatch(request.run_context.data.dispatch_ctx());
        let (_, block_reason) = security
            .screen_new_user_row(&mut request.history)
            .await
            .map_err(|error| driver_failure(error.to_string()))?;
        if let Some(reason) = block_reason {
            return Err(DriverFailure {
                error: RuntimeError::Driver(reason),
                partial: None,
                outcome: None,
            });
        }
        // Snapshot the request boundary before assembling the graph so errors
        // persist only provider-accepted rounds.
        let snapshot = context_mw
            .transcript_snapshot
            .get_or_insert_with(|| {
                Arc::new(std::sync::Mutex::new(
                    crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
                        request_base_len: request.history.len(),
                        ..Default::default()
                    },
                ))
            })
            .clone();
        if let Ok(mut snapshot_state) = snapshot.lock() {
            snapshot_state
                .pricing_model
                .get_or_insert_with(|| self.model_name.clone());
        }
        let sidecar = request.run_context.data.session_sidecar.clone();
        let started = Instant::now();
        let user_message = tinyagents_harness::summarization::last_user_message(&request.history)
            .map(crate::agent::message_convert::user_text_with_markers)
            .unwrap_or_default();
        let context_window = self
            .turn_model_source
            .effective_context_window(&self.model_name)
            .await;
        let turn_models = self
            .turn_model_source
            .build(
                &self.model_name,
                self.temperature,
                context_window,
                request.run_context.data.thread_id.as_deref(),
            )
            .await
            .map_err(driver_error)?;

        let mut messages: Vec<TranscriptMessage> = request
            .history
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect();
        if crate::agent::multimodal::has_image_placeholders(&messages) {
            messages = crate::agent::multimodal::rehydrate_image_placeholders(&messages).await;
        }

        // Harness `ToolSnapshot` remains the model declaration and final
        // allowlist. The actual executable instances must be supplied by the
        // request hook as well: falling back to a long-lived driver's tool
        // registry would let revoked or hidden tools execute under stale
        // authority.
        let tools = request
            .run_context
            .data
            .current_tools
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing request-scoped tools"))?;
        let synthesized_tools = request
            .run_context
            .data
            .current_synthesized_tools
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing request-scoped synthesized tools")
            })?;
        let visible_tool_names: HashSet<String> = request
            .tools
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        ensure_snapshot_tools_are_executable(
            &visible_tool_names,
            &tools,
            &synthesized_tools,
            &request.run_context.data.recorded_tool_names,
        )?;
        // Text dialects need the positional registry to recover P-Format and code-style calls.
        let run_context = request.run_context.data.clone().with_tool_dialect(
            tinyagents_harness::dialect::dispatcher_for_format(self.dispatcher.tool_call_format()),
        );
        let cancellation = request.run_context.cancellation.clone();
        // Boxing avoids worker-stack overflow in debug builds.
        let turn = Box::pin(async move {
            let hosted_base = hosted_base.clone();
            let mut run_context = run_context;
            run_context.workspace = self.workspace.clone().or(run_context.workspace);
            run_context.sandbox_mode = Some(self.sandbox_mode);
            let provider_id = turn_models.provider_id().to_string();
            TurnSpec {
                run_context,
                turn_models,
                provider_id,
                model: &self.model_name,
                history: messages,
                // Durable and synthesized tools follow the declared-snapshot order.
                tool_sets: vec![tools, synthesized_tools],
                // `Some(empty)` is deny-all; `None` re-exposes the driver registry.
                allowed: Some(visible_tool_names),
                max_iterations: self.max_iterations,
                subagent_scope: None,
                context_window,
                run_queue: self.run_queue.clone(),
                message_run_queue: None,
                early_exit_tools: &["ask_user_clarification"],
                pause_at_cap: true,
                max_output_tokens: Some(
                    tinyagents_harness::turn_policy::AGENT_TURN_MAX_OUTPUT_TOKENS,
                ),
                context_mw,
                tool_policy: Some(tool_policy),
                deterministic_cacheable: false,
                defer_turn_completed_to_caller: true,
            }
            .run_hosted(hosted_base, self.agent_id.clone())
            .await
        });
        // A Stop hands provider-accepted rounds back so the runtime retains them (#7715).
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(driver_cancelled_with_snapshot(
                    &snapshot,
                    &sidecar,
                    started.elapsed(),
                    std::mem::take(&mut request.history),
                ));
            }
            result = turn => result,
        };
        let mut outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(driver_error_with_snapshot(
                    error,
                    &snapshot,
                    &sidecar,
                    started.elapsed(),
                ));
            }
        };
        let supplemental_trace = SupplementalTrace::install_for_turn(&sidecar, &mut outcome);

        if outcome.text.trim().is_empty() && outcome.tool_outcomes.is_empty() {
            return Err(driver_error(
                tinyagents_harness::agent_error::AgentError::EmptyProviderResponse {
                    iteration: outcome.model_calls,
                },
            ));
        }
        // A late cap pause still marks an empty exhausted loop as capped.
        outcome.hit_cap |= outcome.text.trim().is_empty()
            && (outcome.model_calls >= self.max_iterations
                || outcome.tool_calls >= self.max_iterations
                || outcome.tool_outcomes.len() >= self.max_iterations);
        // Older hosted-harness fallbacks leave the raw counters unavailable.
        // They only occur when the loop exhausted a tool round without a model
        // conclusion, so normalize them to the same resumable cap state.
        let exhausted_fallback = outcome
            .text
            .starts_with("I finished this turn without writing up a result.");
        outcome.hit_cap |= exhausted_fallback;
        // This fallback is not model-authored; replace it with the resumable checkpoint.
        if exhausted_fallback {
            outcome.wrap_up_injected = false;
        }

        // Keep the close and any required-output repair inside the turn deadline.
        let close_by = request.run_context.data.turn_deadline.map(|deadline| {
            let now = Instant::now();
            now + deadline.close_budget(now)
        });
        // A Stop during close or repair also returns accepted rounds (#7776).
        let accepted_history = request.history.clone();
        let stopped = || {
            driver_cancelled_with_snapshot(
                &snapshot,
                &sidecar,
                started.elapsed(),
                accepted_history.clone(),
            )
        };
        let close = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(stopped()),
            close = grounded_close::close_if_needed(
                &self.turn_model_source,
                &self.model_name,
                self.temperature,
                request.run_context.data.thread_id.as_deref(),
                self.dispatcher.as_ref(),
                &request.history,
                &user_message,
                &outcome,
                close_by.map(|at| at.saturating_duration_since(Instant::now())),
                supplemental_trace.as_ref(),
            ) => close,
        };
        let mut output = close
            .as_ref()
            .map(|close| close.output.clone())
            .unwrap_or_else(|| outcome.text.clone());
        // A tools-only loop may reach the close path after the hosted runtime
        // has already dropped its cap metadata.  Its generic final-summary
        // fallback is not a completed answer; preserve resumability by making
        // the durable reply an explicit checkpoint.
        if outcome.text.trim().is_empty()
            && output.starts_with("I finished this turn without writing up a result.")
        {
            let results = super::super::product_copy::close_results(&outcome.tool_outcomes);
            let rendered = super::super::product_copy::render_close_results(&results, 4_000);
            output = tinyagents_harness::close::checkpoint_with_copy(
                &super::super::product_copy::OPENHUMAN_CLOSE_COPY,
                self.max_iterations,
                &rendered,
                !results.is_empty(),
            );
        }
        let mut history = request.history;
        let conversation = crate::agent::message_convert::provider_messages_from_conversation(
            self.dispatcher.as_ref(),
            &outcome.conversation,
        );
        let mut appended = crate::agent::message_convert::history_to_messages(&conversation);
        if close.is_some() {
            // A blank or breaker-halt terminal row is not the durable answer.
            // Retain every completed tool round and replace only that final
            // assistant row with the verified/deterministic grounded close.
            if appended
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                appended.pop();
            }
            appended.push(Message::assistant(output.clone()));
        }
        history.extend(appended);

        let required_output = request.run_context.data.required_output.clone();
        let classified_halt = outcome
            .breaker_halt
            .as_deref()
            .is_some_and(|reason| reason.starts_with("Stopping after "));
        let time_left = close_by.map(|at| at.saturating_duration_since(Instant::now()));
        let mut repair_usage = grounded_close::RepairUsage::for_model(&self.model_name);
        let required_repair = match required_output.as_ref() {
            Some(contract) if classified_halt || time_left.is_some_and(|left| left.is_zero()) => {
                append_synthesized_block(&mut output, &mut history, contract);
                None
            }
            Some(contract) => {
                let repaired = {
                    let repair = grounded_close::repair_required_output(
                        &self.turn_model_source,
                        &self.model_name,
                        self.temperature,
                        request.run_context.data.thread_id.as_deref(),
                        self.dispatcher.as_ref(),
                        contract,
                        &history,
                        &output,
                        close.is_none() && request.run_context.data.progress.is_some(),
                        request.run_context.data.progress.as_ref(),
                        outcome.model_calls.saturating_add(1) as u32,
                        &mut repair_usage,
                        supplemental_trace.as_ref(),
                    );
                    tokio::pin!(repair);
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Err(stopped()),
                        repaired = async {
                            match time_left {
                                // Bound to what is left so the turn still commits;
                                // on expiry fall back to the deterministic block.
                                Some(left) => tokio::time::timeout(left, &mut repair).await.ok(),
                                None => Some((&mut repair).await),
                            }
                        } => repaired,
                    }
                };
                match repaired {
                    Some(repaired) => repaired,
                    None => {
                        append_synthesized_block(&mut output, &mut history, contract);
                        None
                    }
                }
            }
            None => None,
        };
        if let Some(repaired) = required_repair {
            output = repaired;
            if history
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                history.pop();
            }
            history.push(Message::assistant(output.clone()));
        }

        // This is deliberately an out-of-band observation rather than a
        // second history or transcript.  The runtime only reads it from
        // `after_commit`, so a failed/cancelled turn cannot publish its usage
        // or tool outcomes as if they had become durable.
        {
            // Always billed: a repair that timed out after its call answered
            // still recorded that call here.
            let repair_usage = Some(&repair_usage);
            let mut observed = sidecar
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observed.model_calls = outcome.model_calls
                + close
                    .as_ref()
                    .map(|close| close.usage.model_calls)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.model_calls)
                    .unwrap_or_default();
            observed.tool_calls = outcome.tool_calls;
            observed.input_tokens = outcome.input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.input_tokens)
                    .unwrap_or_default();
            observed.output_tokens = outcome.output_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.output_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.output_tokens)
                    .unwrap_or_default();
            observed.cached_input_tokens = outcome.cached_input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.cached_input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.cached_input_tokens)
                    .unwrap_or_default();
            let mut cost = outcome.cost;
            if let Some(close) = close.as_ref() {
                cost.merge(close.usage.cost);
            }
            if let Some(repair) = repair_usage {
                cost.merge(repair.cost);
            }
            observed.cost = cost;
            let loop_last_call = snapshot
                .lock()
                .ok()
                .map(|guard| (guard.last_call_input_tokens, guard.last_call_output_tokens));
            (
                observed.last_call_input_tokens,
                observed.last_call_output_tokens,
            ) = grounded_close::final_call_tokens(
                loop_last_call,
                close.as_ref().map(|close| &close.usage),
                repair_usage,
            );
            observed.duration = Some(started.elapsed());
            observed.driver_finished_at = Some(std::time::Instant::now());
            observed.tool_outcomes = outcome.tool_outcomes.clone();
            observed.hit_cap = outcome.hit_cap;
            observed.wrap_up_injected = outcome.wrap_up_injected;
            // Preserve the harness stop reason and the wind-down sidecar marker.
            observed.stop = crate::agent::session_host::stop_policy::classify_stop_with_cause(
                outcome.breaker_halt.as_deref(),
                outcome.failure_cause.clone(),
                observed.wind_down,
                outcome.hit_cap,
            );
            if let Some(stop) = &observed.stop {
                tracing::debug!(
                    model = %self.model_name,
                    "[session-driver] turn stopped early; recording on the sidecar {}",
                    stop.status_message()
                );
            }
            observed.resolved_route = outcome.resolved_route.clone();
        };
        // The turn compacted its context: persist the compacted history, so the
        // next turn starts from the checkpoint. The runtime session seals the
        // current generation and opens the next (the history no longer extends
        // it), so the full conversation stays on disk.
        let history = match &outcome.compaction {
            Some(carry) => carry.apply(history),
            None => history,
        };
        Ok(DriverOutcome {
            history,
            output: Some(output),
            partial: None,
            interrupted: outcome.early_exit_tool.is_some() || outcome.hit_cap,
            // `None` lets the runtime derive the outcome from `interrupted`.
            outcome: None,
            tool_outcomes: outcome.tool_outcomes.clone(),
        })
    }
}
