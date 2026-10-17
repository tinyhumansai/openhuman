use super::*;

/// Resumes a `flows_run` that paused at a human-in-the-loop approval gate,
/// continuing it from the durable checkpoint (`thread_id`) with
/// `approvals` newly granted. The UI approval card (B3) calls this once the
/// user decides. See `tinyflows::engine::resume_with_checkpointer`'s doc for
/// the resume mechanics.
///
/// **Host-side approval guard (issue B2 finding #3):** tinyflows 0.2's
/// `resume_with_checkpointer` treats the resume call itself as approval of
/// whatever gate paused the run — its `approvals` argument is advisory only,
/// not enforced inside the crate (`flows_resume(..., approvals: [])` on a
/// paused run would otherwise still complete it). So before ever calling
/// into the engine, this loads the persisted `flow_runs` row for
/// `thread_id` (`flow_runs.id == thread_id`) and requires that `approvals`
/// names at least one of that row's *actually* pending node ids. A run
/// that isn't currently `pending_approval` (already completed, failed, or
/// unknown) is rejected outright — resuming an already-settled thread_id is
/// no longer treated as a harmless no-op, it's a clear error.
pub async fn flows_resume(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    thread_id: &str,
    approvals: Vec<String>,
    rejections: Vec<String>,
) -> Result<Outcome<Value>, String> {
    let flow = store::get_flow(config, flow_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;

    let run_record = store::get_flow_run(config, thread_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!("no paused run to resume: no run recorded for thread '{thread_id}'")
        })?;
    if run_record.flow_id != flow_id {
        return Err(format!(
            "no paused run to resume: run '{thread_id}' belongs to flow '{}', not '{flow_id}'",
            run_record.flow_id
        ));
    }
    if run_record.status != "pending_approval" {
        return Err(format!(
            "no paused run to resume: run '{thread_id}' is not pending approval (status: {})",
            run_record.status
        ));
    }
    // A gate can't be both approved and denied in the same resume — that's an
    // ambiguous instruction, reject it up front.
    if let Some(dup) = approvals.iter().find(|a| rejections.contains(a)) {
        return Err(format!(
            "gate '{dup}' cannot be both approved and rejected in the same resume"
        ));
    }
    // Same host-side guard the approvals path uses (see this fn's doc): the
    // engine trusts whatever the resume delivers, so require that the caller's
    // approvals/rejections actually name a currently-pending gate before ever
    // touching the engine. A denial (issue G4) is enforced the same way — a
    // rejection naming a pending gate is a valid resume just as an approval is.
    let matches_pending = approvals
        .iter()
        .chain(rejections.iter())
        .any(|a| run_record.pending_approvals.contains(a));
    if !matches_pending {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            ?approvals,
            ?rejections,
            pending = ?run_record.pending_approvals,
            "[flows] flows_resume: rejected — caller approvals/rejections name none of the pending gates"
        );
        return Err(format!(
            "no pending approval matches: approvals {approvals:?} / rejections {rejections:?} do \
             not name any of the currently pending gates {:?} for run '{thread_id}'",
            run_record.pending_approvals
        ));
    }

    // T-M1 — stale-approval graph pin. The approval card the user acted on
    // described the graph as it existed at park time. If `save_workflow` (or
    // any other `flows_update`) rewrote the flow's graph while the run sat
    // `pending_approval`, resuming would compile the CURRENT graph against
    // the OLD checkpoint and fire whatever the *new* config of the approved
    // node id now does — under an approval the user never actually saw.
    // `flows_update` deliberately has no in-flight/pending-run guard (that
    // would let a stale park hold a flow hostage for the whole TTL), so this
    // is the fail-closed boundary instead: refuse and settle the run rather
    // than execute. A `None` pin (a legacy row from before this guard
    // existed, or a graph that failed to hash at park time) is treated as
    // "unknown — allow, with a warning" so upgrading mid-park can never
    // strand an otherwise-valid in-flight approval.
    match run_record.graph_hash.as_deref() {
        Some(expected_hash) => {
            let current_hash = compute_graph_hash(&flow.graph, flow.require_approval);
            if current_hash.as_deref() != Some(expected_hash) {
                tracing::warn!(
                    target: "flows",
                    flow_id = %flow_id,
                    %thread_id,
                    expected_hash,
                    current_hash = ?current_hash,
                    "[flows] flows_resume: refusing — the flow's graph changed after this run \
                     parked (T-M1 stale-approval guard)"
                );
                // Settle the row FIRST and treat the guarded write as the
                // authority, exactly as `flows_cancel_run` does (see its
                // ORDER MATTERS note) — this refusal runs BEFORE this call
                // claims the run, so a concurrent resume can legitimately own
                // it by now:
                //
                //   1. Resume B reads the flow and computes a matching hash.
                //   2. `flows_update` rewrites the flow.
                //   3. Resume A reads it, computes a MISMATCH, and lands here.
                //   4. Resume B wins `mark_run_resuming`, flips the row to
                //      `running`, and starts executing approved side effects.
                //
                // `finish_flow_run_row`'s guard admits `running` as well as
                // `pending_approval`, so a blind write from A would relabel
                // B's live row `cancelled`, overwrite `last_status`, and drop
                // a checkpoint B is actively using. Acting only when the write
                // actually matched keeps A's refusal from touching B's run.
                //
                // A is refused either way: its own view of the graph is stale,
                // so it must never proceed regardless of who owns the row.
                let observed = current_persisted_steps(config, thread_id);
                let settled_by_us = finish_flow_run_row(
                    config,
                    thread_id,
                    flow_id,
                    "cancelled",
                    &observed,
                    &[],
                    Some(GRAPH_CHANGED_SINCE_PARK_ERROR),
                    None,
                );
                if settled_by_us {
                    if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                        tracing::warn!(
                            target: "flows",
                            flow_id = %flow_id,
                            %thread_id,
                            error = %e,
                            "[flows] flows_resume: failed to record run summary (stale-approval refusal)"
                        );
                    }
                    // The checkpoint is for a graph that no longer exists as
                    // approved; drop it rather than leave it resumable against
                    // a future graph edit that happens to hash back to the
                    // same value.
                    drop_checkpoint(config, thread_id).await;
                } else {
                    tracing::info!(
                        target: "flows",
                        flow_id = %flow_id,
                        %thread_id,
                        "[flows] flows_resume: stale-approval refusal did not settle the row — another \
                         resume or cancel owns it now; leaving its status and checkpoint untouched"
                    );
                }
                return Err(GRAPH_CHANGED_SINCE_PARK_ERROR.to_string());
            }
        }
        None => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                "[flows] flows_resume: no graph_hash pinned for this parked run (legacy row \
                 predating the T-M1 guard, or the graph failed to hash at park time) — allowing \
                 the resume without a graph-pin check"
            );
        }
    }

    // A pending checkpoint may have been created before this compatibility
    // gate shipped, so resume is an independent authoritative boundary.
    if let Err(error) = ensure_config_aware_engine_compatible(config, &flow.graph) {
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %rec_err,
                "[flows] flows_resume: failed to record compatibility rejection"
            );
        }
        let observed = current_persisted_steps(config, thread_id);
        finish_flow_run_row(
            config,
            thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(&error),
            None,
        );
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            %error,
            "[flows] flows_resume: rejected — unsupported engine topology"
        );
        return Err(error);
    }
    let compiled = tinyflows::compiler::compile(&flow.graph).map_err(|e| e.to_string())?;
    let config_arc = Arc::new(config.clone());
    // The run acts under its own trusted `Workflow` origin: its capabilities
    // read the origin from this context, not from the calling task's scope.
    let origin = workflow_origin(flow_id, flow.require_approval);
    let run_ctx = ctx.with_turn_origin(origin.clone());
    let caps = crate::flows::tinyflows::build_capabilities(
        &run_ctx,
        config_arc.clone(),
        format!("flow:{flow_id}"),
        Some(FlowRunContext {
            flow_id: flow_id.to_string(),
            run_id: thread_id.to_string(),
        }),
    );
    let checkpointer =
        crate::flows::tinyflows::open_flow_checkpointer(config).map_err(|e| e.to_string())?;

    // Run-lifecycle parity with `flows_run` (R-M1). A resume executes the flow's
    // real approved side effects for up to `FLOW_RUN_TIMEOUT_SECS`, so it needs
    // the same three guards the run path has had since B41/B42 — it had none:
    //
    //  1. `run_registry::register` — without an entry, `flows_cancel_run` saw
    //     `is_in_flight == false`, took its "parked/stale" branch, wrote a
    //     terminal `cancelled` row and dropped the checkpoint out from under
    //     this still-executing resume. Registering makes the cancel take the
    //     signalled branch, which this fn now honours in the `select!` below.
    //  2. `mark_run_resuming` — flips the row off `pending_approval` so the
    //     parked-run TTL sweep stops matching a resume that is actively
    //     running.
    //  3. `RunRowFinalizer` — if this future is dropped mid-await (client
    //     disconnect during the long await), the row is reconciled to
    //     `interrupted` instead of being stranded at its old status.
    //
    // Register BEFORE the status flip for the same reason `flows_run` registers
    // before inserting its row: never let a cancel observe a live-looking row
    // that no registered run owns.
    let (cancel_token, _run_guard) = run_registry::register(thread_id);
    match store::mark_run_resuming(config, thread_id) {
        Ok(true) => {}
        Ok(false) => {
            // The guarded flip matched nothing: the run was cancelled or
            // TTL-expired between the status check above and here. Refuse
            // rather than executing approved side effects for a run that is no
            // longer live.
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                "[flows] flows_resume: run left 'pending_approval' before the resume could claim it — refusing"
            );
            return Err(format!(
                "no paused run to resume: run '{thread_id}' was cancelled or expired before the \
                 resume could start"
            ));
        }
        Err(e) => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %e,
                "[flows] flows_resume: failed to mark run as resuming"
            );
            return Err(e.to_string());
        }
    }
    let finalizer = RunRowFinalizer::new(config_arc, thread_id, flow_id);

    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        approval_count = approvals.len(),
        rejection_count = rejections.len(),
        "[flows] flows_resume: resuming checkpointed run"
    );

    // Same per-run journal as `flows_run`: the resumed execution mints a new
    // tinyagents run id, so its observation slice is read under that id.
    let journal = Arc::new(tinyflows::engine::InMemoryGraphEventJournal::new());
    // Live observer (issue G2): the resumed run fires `on_step_finish` for each
    // node that runs after the interrupt boundary, so downstream steps are
    // persisted + streamed live too, keyed by the same `thread_id`/run row.
    let observer: Arc<dyn tinyflows::observability::RunObserver> = Arc::new(
        crate::flows::tinyflows::observability::FlowRunObserver::new(
            Arc::new(config.clone()),
            flow_id,
            thread_id.to_string(),
        ),
    );
    // `rejections` (issue G4 — deny semantics): a denied gate routes to its
    // `error` port (recovery branch) or, if it has none, fails the run. The
    // empty-rejections case is byte-for-byte the prior approve-only resume.
    //
    // Same flow/run correlation as `flows_run` (see its comment), carried by the
    // capabilities above -- a resumed run can dispatch further tool calls that
    // park, and those parks need `source_context` too.
    let run = with_origin(
        origin,
        tinyflows::engine::resume_with_checkpointer_journaled_observed(
            &compiled,
            &caps,
            checkpointer,
            thread_id,
            approvals,
            rejections,
            journal.clone(),
            &observer,
        ),
    );

    // Terminal-write helper for the two failure arms. Row FIRST, then the
    // best-effort summary — see the settle path below for why the order matters.
    let record_failed = |msg: &str| {
        let observed = current_persisted_steps(config, thread_id);
        finish_flow_run_row(
            config,
            thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(msg),
            None,
        );
        if let Err(e) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %e,
                "[flows] flows_resume: failed to record run summary (run row already finalized)"
            );
        }
    };

    let timed = tokio::time::timeout(std::time::Duration::from_secs(FLOW_RUN_TIMEOUT_SECS), run);
    tokio::pin!(timed);
    // Race the resume against a cancellation signal, exactly as `run_flow_body`
    // does. `biased` checks the cancel arm first so a `flows_cancel_run` landing
    // as the resume settles still wins deterministically.
    let journaled = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::info!(target: "flows", flow_id = %flow_id, %thread_id, "[flows] flows_resume: cancelled mid-resume");
            let observed = current_persisted_steps(config, thread_id);
            finish_flow_run_row(
                config,
                thread_id,
                flow_id,
                "cancelled",
                &observed,
                &[],
                Some("run cancelled"),
                None,
            );
            finalizer.disarm();
            if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_resume: failed to record cancelled run");
            }
            drop_checkpoint(config, thread_id).await;
            return Ok(Outcome::single_log(
                json!({
                    "output": Value::Null,
                    "pending_approvals": Vec::<String>::new(),
                    "thread_id": thread_id,
                    "cancelled": true,
                }),
                format!("flow resume cancelled: {thread_id}"),
            ));
        }
        result = &mut timed => match result {
            Ok(Ok(journaled)) => journaled,
            Ok(Err(e)) => {
                record_failed(&e.to_string());
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, %thread_id, error = %e, "[flows] flows_resume: run failed");
                return Err(e.to_string());
            }
            Err(_elapsed) => {
                let msg = format!("flow resume timed out after {FLOW_RUN_TIMEOUT_SECS}s");
                record_failed(&msg);
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, %thread_id, timeout_secs = FLOW_RUN_TIMEOUT_SECS, "[flows] flows_resume: run timed out");
                return Err(msg);
            }
        },
    };
    let outcome = journaled.outcome;

    let settled = settle_steps(config, thread_id, &outcome.output);
    let (status, error) = finalize_terminal_status(&settled, &outcome.pending_approvals);
    // T-M1: a resumed run can itself re-park at a further gate — pin the
    // (already-verified-current, see the graph-hash check above) graph again
    // so a *second* stale-approval window is guarded exactly like the first.
    let graph_hash = (status == "pending_approval")
        .then(|| compute_graph_hash(&flow.graph, flow.require_approval))
        .flatten();
    // Finalize the run row (and disarm the drop-guard) BEFORE the flow-summary
    // write, matching `flows_run` (R-M3). This used to be inverted here, with
    // `record_run` propagating via `?`: a concurrent flow delete made the
    // summary write fail and returned early, leaving the row stranded at
    // `pending_approval` even though the engine had completed and its side
    // effects had fired — which the TTL sweep would later relabel `cancelled`.
    // The row's terminal state is the correctness-critical write; the summary is
    // best-effort observability.
    finish_flow_run_row(
        config,
        thread_id,
        flow_id,
        status,
        &settled,
        &outcome.pending_approvals,
        error.as_deref(),
        graph_hash.as_deref(),
    );
    finalizer.disarm();
    if let Err(e) = store::record_run(config, flow_id, status) {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            status,
            error = %e,
            "[flows] flows_resume: failed to record run summary (run row already finalized)"
        );
    }
    export_run_to_langfuse(
        config,
        &flow.name,
        flow_id,
        thread_id,
        status,
        FlowRunTrigger::Resume,
        &journal,
        &journaled.graph_run_ids.run_id,
    )
    .await;
    notify_pending_approval(&flow, thread_id, &outcome.pending_approvals);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        status,
        pending_approvals = outcome.pending_approvals.len(),
        "[flows] flows_resume: finished"
    );

    Ok(Outcome::single_log(
        json!({
            "output": outcome.output,
            "pending_approvals": outcome.pending_approvals,
            "thread_id": thread_id,
        }),
        format!("flow resume {status}"),
    ))
}

// The T-M1 stale-approval guard's content pin. Persisted on a run row the moment
// it parks at `pending_approval` and recompared against the **current** flow
// before a resume executes (see `flows_resume`'s doc), so a rewrite between park
// and resume is detected instead of silently firing the new configuration under
// the old approval. It covers the graph AND `require_approval`: the flag becomes
// the `AgentTurnOrigin` for the whole resumed execution and `false` auto-allows
// every `external_effect` tool call, and it is settable without touching the
// graph. The algorithm and its fixed-vector test live in `tinyflows-catalog`
// because the digest is a persisted format.
pub(super) use tinyflows_catalog::graph_hash::compute_graph_hash;
