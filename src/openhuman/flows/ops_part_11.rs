
/// [`flows_build`] with caller-specific tools removed in addition to the
/// standard streaming/headless safety lists.
///
/// This is intentionally crate-private: product surfaces use [`flows_build`]'s
/// normal builder belt. Host integrations that add their own persistence
/// boundary can hide tools that would bypass that boundary.
pub(crate) async fn flows_build_with_extra_hidden_tools(
    config: &Config,
    req: crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest,
    stream: Option<FlowStreamTarget>,
    extra_hidden_tools: &[&str],
) -> Result<RpcOutcome<Value>, String> {
    use crate::openhuman::agent::Agent;
    use crate::openhuman::flows::agents::workflow_builder::builder_prompt::render_prompt;

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
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise agent registry: {e}"))?;

    // Issue #4868 — the session builder (`build_session_agent_inner`) now
    // resolves the per-agent iteration cap from the `workflow_builder`
    // `AgentDefinition` itself (`iteration_policy = "extended"` ->
    // `effective_max_iterations()` = 50), so no override is needed here.
    let mut agent = Agent::from_config_for_agent(config, "workflow_builder")
        .map_err(|e| format!("failed to build workflow_builder agent: {e:#}"))?;
    agent.set_agent_definition_name("workflow_builder".to_string());

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
    let approval_gate_active =
        crate::openhuman::security::approval::ApprovalGate::try_global().is_some();
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
            let run = with_origin(
                origin,
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx,
                    APPROVAL_COPILOT_STREAM_CONTEXT.scope((), agent.run_single(&prompt)),
                ),
            );
            let run =
                tokio::time::timeout(std::time::Duration::from_secs(FLOW_BUILD_TIMEOUT_SECS), run);
            let run = crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
                target.thread_id.clone(),
                run,
            );

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
    let proposal = extract_workflow_proposal(agent.history());

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
        let fallback = build_trail_off_fallback(agent.history());
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

/// Heuristic: does `text` already contain a clear, answerable question in its
/// final paragraph? Conservative by design (issue: builder convergence) — a
/// false negative (an actual question this misses) no longer discards the
/// model's text (see `combine_trail_off_fallback`), so the safe failure mode
/// stays "add a guaranteed question on top", never "under-detect and stay
/// silent".
///
/// Regression (#4887 follow-up): the original version only checked for a `?`
/// at the very end of the text / last line, which false-negatived on the
/// extremely common LLM pattern "What's X? You can find it at Y." — a real
/// question immediately followed by a trailing instructional sentence. The
/// backstop then clobbered a specific, answerable question with a generic
/// fallback. To catch that shape, this now also scans the LAST non-empty
/// paragraph for a `?` that isn't inside inline code or a fenced code block
/// (so a literal `?` in a code sample, e.g. `WHERE id = ?`, doesn't count).
///
/// Note: the trailing-noise strip below deliberately does NOT include the
/// backtick. Stripping a trailing backtick would peel off the CLOSING
/// delimiter of a code span whose last character is `?` (e.g. `` `id = ?` ``
/// at the very end of the text), exposing that `?` as if it were a bare
/// trailing question mark and defeating the code guard entirely.
fn text_looks_like_question(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_end_matches(['"', '\'', ')', ']', '*', '_', '.'])
        .trim_end();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('?') {
        return true;
    }
    // The question may not be the literal last character (trailing markdown
    // like a closing code fence or list marker on its own line) — fall back
    // to the last non-blank line.
    if trimmed
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .is_some_and(|last_line| last_line.trim_end().ends_with('?'))
    {
        return true;
    }
    // Final-paragraph scan: a question can sit mid-paragraph, followed by a
    // further trailing sentence on the SAME line/paragraph ("...ID? You can
    // find it under Profile > Copy member ID."). Take the last non-blank
    // paragraph and accept it if it contains a `?` that isn't inside inline
    // code / a code fence.
    last_paragraph(trimmed)
        .as_deref()
        .is_some_and(question_mark_outside_code)
}

/// Returns the last non-blank paragraph of `text` — a maximal run of
/// consecutive non-blank lines, working backward from the end and skipping
/// any trailing blank lines first. `None` if `text` has no non-blank lines.
///
/// CodeRabbit review follow-up: this used to split on the literal `"\n\n"`
/// byte sequence, which mishandles two real shapes:
/// - **CRLF input** (`"question?\r\n\r\nstatus"`): the separator is
///   `"\r\n\r\n"`, not `"\n\n"`, so the whole text was treated as ONE
///   paragraph — an earlier question could then suppress the fallback for a
///   trailing non-question status paragraph.
/// - **Whitespace-only separator lines** (`"question?\n \nstatus"` — a blank
///   line that isn't perfectly empty): same failure, same reason.
///
/// Working line-by-line via [`str::lines`] (which normalizes CRLF) and
/// treating any all-whitespace line as blank fixes both.
fn last_paragraph(text: &str) -> Option<String> {
    let mut collected: Vec<&str> = Vec::new();
    for line in text.lines().rev() {
        if line.trim().is_empty() {
            if collected.is_empty() {
                continue; // still skipping trailing blank lines
            }
            break; // blank line marks the start of the paragraph above
        }
        collected.push(line);
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}

/// Does `text` contain at least one *sentence-terminal* `?` that isn't
/// inside a backtick-delimited code span (inline code like `` `U...` `` or a
/// fenced block like `` ``` ``)? Follows the CommonMark code-span rule: a
/// *run* of one or more consecutive backticks opens a span, and that span is
/// closed only by the next run of the SAME length — a shorter or longer run
/// of backticks encountered while inside a span is just literal backtick
/// characters, not a delimiter.
///
/// CodeRabbit review follow-up: an earlier version tracked a running
/// per-character backtick COUNT and used its parity (even = outside code).
/// That misclassifies any multi-backtick span whose delimiter is more than
/// one backtick — e.g. ``` ``SELECT ? FROM t`` ``` opens with a 2-backtick
/// run (count 0→2, even → looks "outside" again immediately), so the `?`
/// inside a valid double-backtick span was wrongly treated as outside code.
/// Tracking delimiter run LENGTH (not raw backtick count) fixes this while
/// still handling the common single-backtick and triple-backtick-fence
/// cases, since those are just the run-length-1 and run-length-3 instances
/// of the same rule.
///
/// Codex review follow-up: a bare `?` outside code isn't necessarily a real
/// question — a status line like "Checked https://api.example/search?q=foo
/// and got 403." has one mid-token, in a URL query string. Counting that
/// would flip `text_looks_like_question` to `true` and skip
/// `combine_trail_off_fallback` entirely, leaving the user with an
/// unanswerable status note — exactly the failure mode this backstop exists
/// to prevent. So each candidate `?` is additionally required to be
/// sentence-terminal via [`is_sentence_terminal_question_mark`].
fn question_mark_outside_code(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    // `Some(n)` while scanning is inside a code span opened by a run of `n`
    // backticks; that span closes only on the next run of exactly `n`.
    let mut open_run_len: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let start = i;
            while i < chars.len() && chars[i] == '`' {
                i += 1;
            }
            let run_len = i - start;
            open_run_len = match open_run_len {
                None => Some(run_len),
                Some(n) if n == run_len => None,
                Some(n) => Some(n), // mismatched run length: still inside the span
            };
            continue;
        }
        if chars[i] == '?'
            && open_run_len.is_none()
            && is_sentence_terminal_question_mark(&chars, i)
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Is the `?` at `chars[index]` sentence-terminal — i.e. does it read as an
/// actual question mark rather than a character that merely happens to be a
/// `?` mid-token (a URL query string like `search?q=foo`, a shell glob,
/// etc.)? Skips over any immediately-following closing quote/bracket
/// punctuation (`"`, `'`, right single/double quotes, `)`, `]`) and requires
/// what remains to be whitespace or the end of the text — the shape a `?`
/// takes at the end of a real sentence or clause.
fn is_sentence_terminal_question_mark(chars: &[char], index: usize) -> bool {
    let mut i = index + 1;
    while let Some(&c) = chars.get(i) {
        if matches!(c, '"' | '\'' | '\u{2019}' | '\u{201D}' | ')' | ']') {
            i += 1;
            continue;
        }
        return c.is_whitespace();
    }
    true // '?' was the last character in the paragraph.
}

/// Builder-authoring tools whose result body can explain a trail-off — the
/// authoring belt `dry_run_workflow`/`validate_workflow`/`propose_workflow`/
/// `revise_workflow`/`edit_workflow`/`save_workflow` all report either a hard
/// gate rejection (`ToolResult::error`) or a self-reported broken-graph
/// result (`"ok": false` in a successful body), so a plain-text read-only
/// tool's output is never misattributed as the blocker.
const TRAIL_OFF_BLOCKER_TOOLS: &[&str] = &[
    "dry_run_workflow",
    "validate_workflow",
    "propose_workflow",
    "revise_workflow",
    "edit_workflow",
    "save_workflow",
];

/// Synthesizes a guaranteed, user-facing fallback for a trail-off turn (no
/// proposal, not capped, no run error, and the model's own text isn't a
/// question). Scans the run's tool history for the last builder-tool result
/// that looks like a blocker (a hard-gate rejection, or a `dry_run_workflow`/
/// `validate_workflow` report with `"ok": false`) and asks the user about it;
/// falls back to a generic "what should I focus on" question when no such
/// blocker is found (the model may have simply stopped with nothing to point
/// to).
fn build_trail_off_fallback(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> String {
    match last_builder_tool_blocker(history) {
        Some(blocker) => format!(
            "I wasn't able to finish building this workflow. Here's where I got stuck:\n\n{blocker}\n\n\
             Could you tell me how you'd like me to resolve that, or share more detail about what's needed here?"
        ),
        None => "I wasn't able to finish building this workflow in this turn. Could you describe \
                  what you'd like in more detail, or tell me which part to focus on?"
            .to_string(),
    }
}

/// Combines the guaranteed trail-off `fallback` question with the model's own
/// `original` text instead of discarding it (#4887 follow-up, Change 2). Even
/// after loosening `text_looks_like_question`, a future false negative must
/// never destroy the model's words — it should only ever ADD the guaranteed
/// question on top. The `fallback` is prepended (so the user sees the
/// actionable question first) and the original is kept below a divider for
/// context. When `original` is empty/whitespace-only (a genuine silent
/// turn — there's nothing to preserve), returns the fallback alone rather
/// than prepending an empty divider.
fn combine_trail_off_fallback(fallback: &str, original: &str) -> String {
    let trimmed_original = original.trim();
    if trimmed_original.is_empty() {
        fallback.to_string()
    } else {
        format!("{fallback}\n\n---\n\n{trimmed_original}")
    }
}

/// Scans `history` in reverse for the last result from a
/// [`TRAIL_OFF_BLOCKER_TOOLS`] call that reads as a failure — a plain-text
/// error message (gate rejection), or a JSON body with `"ok": false` — and
/// returns a truncated, human-readable description of it. Tool names are
/// resolved by correlating each `ToolResults` entry's `tool_call_id` back to
/// the `AssistantToolCalls` message that issued it, so this never
/// misattributes an unrelated read-only tool's plain-text output as a
/// blocker.
fn last_builder_tool_blocker(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> Option<String> {
    use crate::openhuman::agent::messages::ConversationMessage;

    let mut call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for message in history {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = message {
            for call in tool_calls {
                call_names.insert(call.id.clone(), call.name.clone());
            }
        }
    }

    for message in history.iter().rev() {
        let ConversationMessage::ToolResults(results) = message else {
            continue;
        };
        for result in results.iter().rev() {
            let Some(name) = call_names.get(&result.tool_call_id) else {
                continue;
            };
            if !TRAIL_OFF_BLOCKER_TOOLS.contains(&name.as_str()) {
                continue;
            }
            // This is the MOST RECENT authoring-belt tool result in the
            // turn (results are scanned newest-first). Whatever it reads as
            // is authoritative: a success/progress result here means any
            // earlier failure from the same tool was already resolved
            // within this turn, so we must stop at this result rather than
            // keep walking backward and surfacing a stale, already-fixed
            // blocker (see review discussion on this PR).
            return describe_tool_result_blocker(&result.content)
                .map(|desc| crate::openhuman::util::truncate_with_ellipsis(&desc, 500));
        }
    }
    None
}
