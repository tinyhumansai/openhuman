use super::*;

/// Runs the `workflow_builder` agent for one authoring turn and returns its
/// proposal, invoking it as a first-class backend agent (exactly like the Flow
/// Scout `flows_discover`) rather than routing a hand-crafted delegate prompt
/// through the chat orchestrator.
///
/// The turn's natural-language brief is rendered **server-side** from the
/// structured [`BuilderRequest`](crate::flows::agents::workflow_builder::builder_prompt::BuilderRequest)
/// (create / revise / repair / build). The agent ends by calling
/// `propose_workflow` / `revise_workflow` / `save_workflow`; we capture the
/// resulting `{ type: "workflow_proposal", … }` payload from the run's tool
/// history and return it alongside the agent's final assistant text.
///
/// Persistence stays with the agent's tools: `propose`/`revise` never persist;
/// `save_workflow` (only reachable in `build` mode with a real `flow_id`)
/// writes onto an existing flow. This op never enables or runs a flow.
pub async fn flows_build(
    config: &Config,
    req: crate::flows::agents::workflow_builder::builder_prompt::BuilderRequest,
    stream: Option<FlowStreamTarget>,
) -> Result<RpcOutcome<Value>, String> {
    flows_build_with_extra_hidden_tools(config, req, stream, &[]).await
}

/// [`flows_build`] with caller-specific tools removed in addition to the
/// standard streaming/headless safety lists.
///
/// This is intentionally crate-private: product surfaces use [`flows_build`]'s
/// normal builder belt. Host integrations that add their own persistence
/// boundary can hide tools that would bypass that boundary.
pub(crate) async fn flows_build_with_extra_hidden_tools(
    config: &Config,
    req: crate::flows::agents::workflow_builder::builder_prompt::BuilderRequest,
    stream: Option<FlowStreamTarget>,
    extra_hidden_tools: &[&str],
) -> Result<RpcOutcome<Value>, String> {
    use crate::agent::OpenHumanSessionHost;
    use crate::flows::agents::workflow_builder::builder_prompt::render_prompt;

    // Reject invalid turns (e.g. a `build` with no `flow_id`) before we render a
    // brief that would tell the agent to save onto nothing.
    req.validate()?;

    let prompt = render_prompt(&req);
    tracing::info!(
        target: "flows",
        mode = ?req.mode,
        has_graph = req.graph.is_some(),
        flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
        streaming = stream.is_some(),
        "[flows] flows_build: starting workflow_builder turn"
    );

    // The registry must be initialised before building a named builtin agent
    // (idempotent — mirrors `flows_discover`).
    crate::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise agent registry: {e}"))?;

    // Issue #4868 — the session builder (`build_session_agent_inner`) now
    // resolves the per-agent iteration cap from the `workflow_builder`
    // `AgentDefinition` itself (`iteration_policy = "extended"` ->
    // `effective_max_iterations()` = 50), so no override is needed here.
    let mut agent = OpenHumanSessionHost::from_config_for_agent(config, "workflow_builder")
        .map_err(|e| format!("failed to build workflow_builder agent: {e:#}"))?;
    agent.set_agent_definition_name("workflow_builder".to_string());
    start_builder_turn_clean(&mut agent);

    // Restrict the visible run-advancing tools per path (PR3:
    // flows-copilot-live-run-approval). Streaming (copilot pane, real approval
    // surface below) only hides the always-hidden `run_workflow`; headless
    // (CLI / tests / no chat thread) keeps the full historical hide-list
    // (issue #4593 / #4881) since there is no routable approval surface there.
    //
    // The reduced (copilot) hide-list is safe ONLY when the process-global
    // `ApprovalGate` is actually installed to park the unhidden
    // `run_flow`/`resume_flow_run`. `flows_build` is a public RPC and the gate
    // can be opted out (`OPENHUMAN_APPROVAL_GATE=0` on CLI/docker leaves
    // `ApprovalGate::try_global()` == `None`; desktop always installs it) — and
    // `ApprovalSecurityMiddleware` skips interception entirely when the gate is
    // absent, so the WebChat origin below would NOT park and the unhidden
    // live-run tools would execute unapproved. Fall back to the full hide-list
    // whenever the gate is not installed, regardless of `stream`. (codex #5090)
    let approval_gate_active = crate::security::approval::ApprovalGate::try_global().is_some();
    if stream.is_some() && approval_gate_active {
        restrict_builder_toolset_for_copilot(&mut agent);
    } else {
        if stream.is_some() {
            tracing::warn!(
                target: "flows",
                "[flows] flows_build: streaming turn but no ApprovalGate installed \
                 (OPENHUMAN_APPROVAL_GATE off / headless) — keeping the full live-run \
                 hide-list so run_flow/resume_flow_run cannot execute unapproved"
            );
        }
        restrict_builder_toolset(&mut agent);
    }
    if !extra_hidden_tools.is_empty() {
        tracing::debug!(
            target: "flows",
            hidden = ?extra_hidden_tools,
            "[flows] flows_build: applying caller-specific hidden tools"
        );
        agent.hide_tools(extra_hidden_tools);
    }

    // When a chat thread is attached (the copilot pane), stream the builder turn
    // into it exactly like an interactive turn — text/tool deltas and the
    // `propose_workflow` tool result the frontend renders as a proposal card.
    // Best-effort — with no target the run stays headless (CLI / tests).
    if let Some(target) = &stream {
        attach_flow_progress_bridge(&mut agent, target, "flows_build", config);
    }

    // Run to completion, bounded by a wall-clock timeout. PR3
    // (flows-copilot-live-run-approval): the origin now depends on whether a
    // chat thread is attached.
    //
    // - Streaming (copilot pane): run under `AgentTurnOrigin::WebChat` with
    //   `APPROVAL_CHAT_CONTEXT` scoped alongside it — the identical
    //   double-scope pattern `web_chat::ops::run_turn_under_cancel_and_deadline`
    //   uses for a real interactive chat turn. The approval gate then PARKS
    //   (rather than auto-allows) any `external_effect` tool call instead of
    //   failing closed, and the resulting `ApprovalRequested` event routes back
    //   to this thread (`client_id: "system"` — every client auto-joins that
    //   broadcast room, matching the progress bridge above) for the existing
    //   `ApprovalRequestCard` to render. The run is additionally wrapped in the
    //   thread-id scope so descendant turns tag their trace + socket events
    //   with this thread.
    // - Headless (CLI / tests / no chat thread): unchanged `AgentTurnOrigin::Cli`
    //   — the gate auto-allows `external_effect` tools under that origin, which
    //   is why `restrict_builder_toolset` above must keep the full hide-list on
    //   this path; there is no routable approval surface here to park against.
    // Outcome of racing the run future against its wall-clock timeout and
    // (streaming only) a user Stop-button cancellation. Kept as one enum so
    // both branches below (and the settle match after) share one shape.
    enum BuildRunOutcome {
        /// The agent run itself finished (or errored) before the timeout or a
        /// cancel raced it.
        Ran(anyhow::Result<String>),
        /// `FLOW_BUILD_TIMEOUT_SECS` elapsed first.
        TimedOut,
        /// The user cancelled the turn (`flows_build_cancel`) before it
        /// finished. Streaming-only — the headless/CLI branch never
        /// registers a token, so it can never produce this.
        Cancelled,
    }

    let timed = match &stream {
        Some(target) => {
            let origin = AgentTurnOrigin::WebChat {
                thread_id: target.thread_id.clone(),
                client_id: "system".to_string(),
                request_id: Some(target.request_id.clone()),
            };
            let chat_ctx = ApprovalChatContext {
                thread_id: target.thread_id.clone(),
                client_id: "system".to_string(),
            };
            tracing::info!(
                target: "flows",
                thread_id = %target.thread_id,
                request_id = %target.request_id,
                "[flows] flows_build: streaming copilot turn — WebChat origin + \
                 APPROVAL_CHAT_CONTEXT scoped, live-run tools park for approval instead \
                 of auto-allowing (shortened to COPILOT_APPROVAL_TTL via \
                 APPROVAL_COPILOT_STREAM_CONTEXT)"
            );
            // `APPROVAL_COPILOT_STREAM_CONTEXT` scopes alongside the existing
            // chat context so any `run_flow`/`resume_flow_run` park raised by
            // this turn is clamped to the shorter `COPILOT_APPROVAL_TTL`
            // instead of the gate's full ten-minute default — a stale park on
            // a copilot pane the user may have already navigated away from
            // shouldn't idle that long. Main-chat turns never scope this, so
            // they are unaffected.
            agent.set_thread_id(Some(target.thread_id.as_str()));
            let run = with_origin(
                origin,
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx,
                    APPROVAL_COPILOT_STREAM_CONTEXT.scope((), agent.run_single(&prompt)),
                ),
            );
            let run =
                tokio::time::timeout(std::time::Duration::from_secs(FLOW_BUILD_TIMEOUT_SECS), run);

            // Register this turn's cancellation token BEFORE racing the run,
            // so a `flows_build_cancel` call landing the instant this turn
            // starts can never miss the registration window. The run stays
            // awaited INLINE (never spawned) — spawning it would drop the
            // task-local `with_origin` / `APPROVAL_CHAT_CONTEXT.scope` /
            // `APPROVAL_COPILOT_STREAM_CONTEXT.scope` / thread-id scope
            // context above, which the approval gate + tracing depend on.
            // `tokio::select!` races the two futures on THIS task instead, so
            // every one of those scopes stays attached to the winning arm.
            let token = CancellationToken::new();
            build_registry::register_build_turn(
                target.thread_id.clone(),
                Some(target.request_id.clone()),
                token.clone(),
            );
            let outcome = tokio::select! {
                r = run => match r {
                    Ok(inner) => BuildRunOutcome::Ran(inner),
                    Err(_) => BuildRunOutcome::TimedOut,
                },
                _ = token.cancelled() => {
                    tracing::debug!(
                        target: "flows",
                        thread_id = %target.thread_id,
                        request_id = %target.request_id,
                        "[flows] flows_build: cancelled by user"
                    );
                    BuildRunOutcome::Cancelled
                }
            };
            // Unconditional — covers every exit the `select!` above can take
            // (ran to completion, errored, timed out, or was cancelled); there
            // is no early return between `register_build_turn` and here that
            // could skip it.
            build_registry::unregister_build_turn(&target.thread_id, Some(&target.request_id));
            outcome
        }
        None => {
            tracing::debug!(
                target: "flows",
                "[flows] flows_build: headless/CLI turn — Cli origin, approval gate \
                 auto-allows external_effect tools (run-advancing tools stay hidden)"
            );
            let run = with_origin(AgentTurnOrigin::Cli, agent.run_single(&prompt));
            match tokio::time::timeout(std::time::Duration::from_secs(FLOW_BUILD_TIMEOUT_SECS), run)
                .await
            {
                Ok(inner) => BuildRunOutcome::Ran(inner),
                Err(_) => BuildRunOutcome::TimedOut,
            }
        }
    };
    let (assistant_text, run_error, cancelled) = match timed {
        BuildRunOutcome::Ran(Ok(text)) => (text, None, false),
        BuildRunOutcome::Ran(Err(e)) => {
            tracing::warn!(target: "flows", error = %e, "[flows] flows_build: agent run failed");
            (
                String::new(),
                Some(format!("workflow_builder run failed: {e:#}")),
                false,
            )
        }
        BuildRunOutcome::TimedOut => {
            tracing::warn!(
                target: "flows",
                timeout_secs = FLOW_BUILD_TIMEOUT_SECS,
                "[flows] flows_build: agent run timed out"
            );
            (
                String::new(),
                Some(format!(
                    "workflow_builder run timed out after {FLOW_BUILD_TIMEOUT_SECS}s"
                )),
                false,
            )
        }
        // A user Stop is not an error (`run_error = None`) — it must not be
        // reported as a failed turn, nor fall into the trail-off backstop
        // below that synthesizes a "continue?" question for a turn that
        // quietly ran out of steam; a deliberate cancel is neither.
        BuildRunOutcome::Cancelled => (String::new(), None, true),
    };

    // Capture the proposal from the run's tool history (propose/revise/save all
    // emit the same self-describing `{ type: "workflow_proposal", … }` payload).
    // Extracted BEFORE the stream is finalized below (issue: builder
    // convergence): the trail-off backstop needs `proposal`/`capped` to decide
    // whether to override `assistant_text`, and the streamed copilot-pane chat
    // bubble must render the SAME (possibly-overridden) text as the RPC
    // response — the frontend renders from the stream, not the return value,
    // so patching only the latter would still leave an interactive user
    // staring at the original silent/status-only text.
    let runtime_history = agent.history();
    let proposal = extract_workflow_proposal(&runtime_history);

    // A user-cancelled turn settles here, clean and separate from the
    // error/trail-off paths below: `finalize_flow_stream` gets an `Ok(...)` (a
    // Stop is not an error) so the copilot pane receives the same `chat_done`
    // terminal event a normal completion would — `ChatRuntimeProvider` ends
    // the inference turn / detaches the streaming state on that event exactly
    // as it does for any other settle, so nothing is left dangling on the FE.
    // Whatever `proposal`/`assistant_text` the turn produced before the
    // cancel raced it (e.g. it had already called `propose_workflow`) is
    // still returned — cancelling doesn't discard partial progress.
    if cancelled {
        if let Some(target) = &stream {
            let terminal: Result<String, String> = Ok(assistant_text.clone());
            finalize_flow_stream(target, &terminal, &prompt).await;
        }
        tracing::info!(
            target: "flows",
            flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
            has_proposal = proposal.is_some(),
            "[flows] flows_build: workflow builder turn cancelled by user"
        );
        return Ok(RpcOutcome::single_log(
            json!({
                "proposal": proposal,
                "assistant_text": assistant_text,
                "error": Value::Null,
                "capped": false,
                "trail_off": false,
            }),
            "workflow builder turn cancelled by user",
        ));
    }

    // A run that both errored AND produced no proposal is a hard failure; a run
    // that proposed before erroring still returns the proposal for review.
    if proposal.is_none() {
        if let Some(err) = &run_error {
            if let Some(target) = &stream {
                let terminal: Result<String, String> = Err(err.clone());
                finalize_flow_stream(target, &terminal, &prompt).await;
            }
            return Err(format!("workflow_builder produced no proposal: {err}"));
        }
    }

    // (B34) Whether this turn paused because it hit `max_tool_iterations`
    // rather than finishing naturally (asking a question, or proposing). A
    // capped turn with no proposal renders a raw checkpoint ("Done so far /
    // Next steps") that's indistinguishable, in the response shape alone,
    // from the agent voluntarily asking a clarifying question — `capped`
    // gives the frontend the explicit signal to render a "Continue building"
    // card instead. Scoped to `proposal.is_none()`: a turn that hit the cap
    // but still squeezed out a proposal (the checkpoint fires before the
    // final `propose_workflow` call in that ordering) has nothing left to
    // continue.
    let hit_cap = agent.last_turn_hit_cap();
    let capped = hit_cap && proposal.is_none();

    // Terminal-state guarantee (builder convergence fix): a turn can end
    // "naturally" (no more tool calls, not capped, no run error) yet still
    // produce neither a proposal nor a real question — the model ran out of
    // steam mid-build and left a status dump ("Done so far: checked
    // connections…") as its final reply. `prompt.md` tells the model to
    // always end a building turn in a proposal or a question, but a prompt
    // rule can be silently ignored; this is the fail-closed backend backstop
    // that makes it a hard invariant regardless of model behavior — the user
    // is NEVER left with silence or an unanswerable status note.
    let trail_off = !capped && proposal.is_none() && run_error.is_none();
    let assistant_text = if trail_off && !text_looks_like_question(&assistant_text) {
        let runtime_history = agent.history();
        let fallback = build_trail_off_fallback(&runtime_history);
        let combined = combine_trail_off_fallback(&fallback, &assistant_text);
        tracing::warn!(
            target: "flows",
            flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
            original_len = assistant_text.len(),
            fallback_len = fallback.len(),
            combined_len = combined.len(),
            "[flows] flows_build: trail-off detected (no proposal, no cap, no question) — \
             guaranteeing a fallback question while preserving the model's original text"
        );
        combined
    } else {
        assistant_text
    };

    // Emit the terminal chat event so a client viewing the copilot thread stops
    // "processing" and finalizes the assistant bubble (the bridge streams only
    // intermediate deltas). Success delivers `chat_done`; a run error delivers
    // `chat_error`. The blocking return below is unchanged. Uses the
    // (possibly trail-off-overridden) `assistant_text` above.
    if let Some(target) = &stream {
        let terminal: Result<String, String> = match &run_error {
            None => Ok(assistant_text.clone()),
            Some(err) => Err(err.clone()),
        };
        finalize_flow_stream(target, &terminal, &prompt).await;
    }

    tracing::info!(
        target: "flows",
        flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
        has_proposal = proposal.is_some(),
        hit_cap,
        capped,
        trail_off,
        "[flows] flows_build: workflow_builder turn complete"
    );
    Ok(RpcOutcome::single_log(
        json!({
            "proposal": proposal,
            "assistant_text": assistant_text,
            "error": run_error,
            "capped": capped,
            "trail_off": trail_off,
        }),
        "workflow builder turn complete",
    ))
}

/// Cancel the in-flight `flows_build` (Workflow Copilot) turn streaming into
/// `thread_id`, scoped by `request_id` — the real, working half of the
/// composer's Stop button (issue: the original FE-only version hid the
/// button but never touched the running turn, since `flows_build` runs the
/// agent inline and never registers in `web_chat::IN_FLIGHT` or
/// `task_dispatcher::ACTIVE_RUNS`).
///
/// When `request_id` is `Some`, the cancel only fires if it matches the turn
/// currently registered on `thread_id` — a stale Stop click for a
/// superseded/earlier request can't kill a newer turn that has since started
/// on the same thread (mirrors `task_dispatcher::cancel_session_scoped`,
/// #4760). `None` cancels whatever turn is on the thread. Returns whether a
/// turn was found and signalled; `false` is not an error — it just means
/// nothing was in flight to cancel (already settled, or never started).
pub async fn flows_build_cancel(
    thread_id: &str,
    request_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled = build_registry::cancel_build_turn_scoped(thread_id, request_id);
    tracing::info!(
        target: "flows",
        thread_id,
        request_id = request_id.unwrap_or("<none>"),
        cancelled,
        "[flows] flows_build_cancel: cancel request handled"
    );
    Ok(RpcOutcome::single_log(
        json!({ "cancelled": cancelled }),
        if cancelled {
            "workflow builder turn cancellation requested"
        } else {
            "no in-flight workflow builder turn to cancel"
        },
    ))
}

/// Scans an agent run's conversation history for the workflow proposal a builder
/// tool emitted. `propose_workflow` / `revise_workflow` / `save_workflow` all
/// return a self-describing `{ "type": "workflow_proposal", … }` JSON string as
/// their tool result, so we match on that (the same gate the frontend uses) and
/// return the LAST one — the most recent proposal in the turn.
pub(super) fn extract_workflow_proposal(
    history: &[crate::agent::messages::ConversationMessage],
) -> Option<Value> {
    use crate::agent::messages::ConversationMessage;
    let mut latest = None;
    for message in history {
        if let ConversationMessage::ToolResults(results) = message {
            for result in results {
                if let Ok(value) = serde_json::from_str::<Value>(&result.content) {
                    if value.get("type").and_then(Value::as_str) == Some("workflow_proposal") {
                        latest = Some(value);
                    }
                }
            }
        }
    }
    latest
}

/// Keep the builder's first turn from auto-resuming another flow's session.
///
/// On an empty history `OpenHumanSessionHost::turn` falls back to `try_load_session_transcript`,
/// which loads the newest transcript for the agent *name*. Every flow's builder
/// shares the name `workflow_builder` and the profile's `session_raw/` dir, so
/// that fallback hands a brand-new flow the most recent builder conversation from
/// whichever flow ran last. Thread-scoped resume, where a host does it, seeds
/// `cached_transcript_messages` and is unaffected. `agent_chat` suppresses the
/// same fallback for the same reason.
pub(crate) fn start_builder_turn_clean(agent: &mut crate::agent::OpenHumanSessionHost) {
    agent.set_next_turn_overrides(crate::agent::session_host::TurnOverrides {
        suppress_transcript_autoload: true,
        ..Default::default()
    });
}
