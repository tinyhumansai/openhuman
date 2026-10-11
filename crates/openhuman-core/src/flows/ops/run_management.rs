use super::*;

/// Cancels a flow run (issue G4), settling it to a terminal `"cancelled"`
/// status and dropping its durable checkpoint so the aborted thread can never
/// be resumed.
///
/// Two cases, distinguished by [`run_registry::cancel`]:
/// - **In-flight** (a `flows_run` / `flows_resume` currently executing its run
///   future): the token is signalled and that run's own cancellation arm writes
///   the terminal row + drops the checkpoint as it unwinds — we don't write the
///   row here, to avoid two writers racing the same `flow_runs` row.
/// - **Parked / stale** (a `pending_approval` run awaiting a human decision, or
///   a `running` row whose task is gone): no live task exists to unwind, so
///   this settles the row terminally itself and drops the checkpoint.
///
/// A run that is already terminal (`completed` / `completed_with_warnings` /
/// `failed` / `cancelled` / `interrupted`) is a clear error, not a silent
/// no-op — otherwise a settled warning run could be overwritten as
/// `"cancelled"`, corrupting the run-honesty status it already recorded, and an
/// already-`interrupted` run (reconciled by the drop-guard / boot sweep, bug
/// B42) could be clobbered back to `"cancelled"`.
pub async fn flows_cancel_run(config: &Config, run_id: &str) -> Result<Outcome<Value>, String> {
    let run = store::get_flow_run(config, run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow run '{run_id}' not found"))?;

    if matches!(
        run.status.as_str(),
        "completed" | "completed_with_warnings" | "failed" | "cancelled" | "interrupted"
    ) {
        return Err(format!(
            "flow run '{run_id}' is already terminal (status: {}) — nothing to cancel",
            run.status
        ));
    }

    let signalled = run_registry::cancel(run_id);
    tracing::info!(
        target: "flows",
        run_id,
        flow_id = %run.flow_id,
        signalled,
        prior_status = %run.status,
        "[flows] flows_cancel_run: cancelling run"
    );

    if signalled {
        // The in-flight run's cancellation arm owns the terminal write + the
        // checkpoint drop; we've signalled it and return. Its settle is
        // eventual (the run future unwinds), so report "requested".
        return Ok(Outcome::single_log(
            json!({ "run_id": run_id, "cancelled": true, "was_in_flight": true }),
            format!("flow run {run_id} cancellation requested"),
        ));
    }

    // Not in flight: settle the row terminally and drop the checkpoint here.
    //
    // ORDER MATTERS (R-M2). The status read above and `run_registry::cancel`
    // are two separate observations, and a live run can settle in the window
    // between them: it writes its own terminal row and deregisters, so
    // `cancel` returns `false` and we arrive here believing the run is merely
    // parked/stale. Writing `cancelled` unconditionally would then relabel a
    // fully-completed run — whose real side effects already fired — and drop a
    // checkpoint that is no longer ours to drop. So attempt the guarded row
    // write FIRST and treat it as the authority: it only matches a still-live
    // row, so `false` means the run settled underneath us. Only once it has
    // won do we record the flow summary and drop the checkpoint.
    let observed = current_persisted_steps(config, run_id);
    let settled_by_us = finish_flow_run_row(
        config,
        run_id,
        &run.flow_id,
        "cancelled",
        &observed,
        &[],
        Some("run cancelled"),
        None,
    );
    if !settled_by_us {
        tracing::info!(
            target: "flows",
            run_id,
            flow_id = %run.flow_id,
            prior_status = %run.status,
            "[flows] flows_cancel_run: run settled concurrently — leaving its terminal status intact"
        );
        return Err(format!(
            "flow run '{run_id}' settled before it could be cancelled — its recorded outcome was \
             left untouched"
        ));
    }
    if let Err(e) = store::record_run(config, &run.flow_id, "cancelled") {
        tracing::warn!(target: "flows", run_id, flow_id = %run.flow_id, error = %e, "[flows] flows_cancel_run: failed to record cancelled status on flow summary");
    }
    drop_checkpoint(config, run_id).await;

    Ok(Outcome::single_log(
        json!({ "run_id": run_id, "cancelled": true, "was_in_flight": false }),
        format!("flow run {run_id} cancelled"),
    ))
}

/// Best-effort drop of a run's durable tinyagents checkpoint thread, so a
/// cancelled (or expired) run can never be resumed from its persisted interrupt
/// boundary. Logged, never fatal — the `flow_runs` row's terminal status is the
/// authoritative "not resumable" signal (the `flows_resume` guard already
/// rejects any non-`pending_approval` status); dropping the checkpoint is
/// belt-and-suspenders that also reclaims the storage.
pub(super) async fn drop_checkpoint(config: &Config, thread_id: &str) {
    match crate::flows::tinyflows::open_flow_checkpointer(config) {
        Ok(checkpointer) => match checkpointer.delete_thread(thread_id).await {
            Ok(()) => {
                tracing::debug!(target: "flows", thread_id, "[flows] dropped durable checkpoint for cancelled/expired run")
            }
            Err(e) => {
                tracing::warn!(target: "flows", thread_id, error = %e, "[flows] failed to drop durable checkpoint")
            }
        },
        Err(e) => {
            tracing::warn!(target: "flows", thread_id, error = %e, "[flows] could not open checkpointer to drop checkpoint");
        }
    }
}

/// Lists the most recent runs for a flow (newest first), for the B3
/// run-history inspector. Runs a lazy parked-run TTL sweep first (see
/// [`sweep_expired_parked_runs`]) so the listing reflects any run that has now
/// aged out of `pending_approval`.
pub async fn flows_list_runs(
    config: &Config,
    flow_id: &str,
    limit: usize,
) -> Result<Outcome<Vec<FlowRun>>, String> {
    sweep_expired_parked_runs(config).await;
    let runs = store::list_flow_runs(config, flow_id, limit).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        runs,
        format!("flow runs listed: {flow_id}"),
    ))
}

/// List the most recent runs across ALL flows, newest first — backs the
/// aggregate "All runs" page. Each returned run carries its `flow_id` so the UI
/// can group/label by workflow.
pub async fn flows_list_all_runs(
    config: &Config,
    limit: usize,
) -> Result<Outcome<Vec<FlowRun>>, String> {
    sweep_expired_parked_runs(config).await;
    let runs = store::list_all_flow_runs(config, limit).map_err(|e| e.to_string())?;
    let count = runs.len();
    Ok(Outcome::single_log(
        runs,
        format!("all flow runs listed: {count} run(s)"),
    ))
}

/// Manually prunes a flow's run history down to the retention cap
/// ([`tinyflows_sqlite::flows::MAX_FLOW_RUNS_PER_FLOW`]), deleting only terminal runs outside the
/// newest-N window. Never removes a `running` or `pending_approval` run — a
/// parked run must survive for a later `flows_resume`. Pruning also happens
/// automatically on every new-run insert; this RPC exposes it for an explicit
/// on-demand sweep (e.g. a maintenance action). Returns the number of runs
/// pruned.
pub async fn flows_prune_runs(config: &Config, flow_id: &str) -> Result<Outcome<Value>, String> {
    let keep = tinyflows_sqlite::flows::MAX_FLOW_RUNS_PER_FLOW;
    let pruned = store::prune_flow_runs(config, flow_id, keep).map_err(|e| e.to_string())?;
    tracing::info!(target: "flows", flow_id, pruned, keep, "[flows] flows_prune_runs: manual retention sweep");
    Ok(Outcome::single_log(
        json!({ "flow_id": flow_id, "pruned": pruned, "kept": keep }),
        format!("flow runs pruned: {flow_id} ({pruned} removed)"),
    ))
}

/// Loads a single flow run record by id (== `thread_id`). Runs the lazy
/// parked-run TTL sweep first so a stale parked run is reported as `cancelled`
/// rather than perpetually `pending_approval`.
pub async fn flows_get_run(config: &Config, run_id: &str) -> Result<Outcome<FlowRun>, String> {
    sweep_expired_parked_runs(config).await;
    let run = store::get_flow_run(config, run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow run '{run_id}' not found"))?;
    Ok(Outcome::single_log(
        run,
        format!("flow run loaded: {run_id}"),
    ))
}

/// Lazy TTL sweep (issue G4): expires every parked `pending_approval` run older
/// than [`FLOW_PARKED_TTL_SECS`] to a terminal `"cancelled"`, updates the flow
/// summary, and drops each expired run's durable checkpoint so it can't be
/// resumed. Mirrors the `approval` domain's expire-on-read idiom
/// (`approval::store::expire_stale`): called at the top of the run-read paths
/// rather than from a dedicated background timer, so it needs no scheduler.
///
/// Best-effort by construction — a sweep failure is logged and swallowed, never
/// failing the read that triggered it. The `flows_resume` status guard already
/// rejects any non-`pending_approval` run, so a swept run is unresumable the
/// instant its row flips, independent of the checkpoint drop.
pub async fn sweep_expired_parked_runs(config: &Config) -> usize {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(FLOW_PARKED_TTL_SECS)).to_rfc3339();
    let now_str = now.to_rfc3339();
    let error_msg = format!("parked run expired after {FLOW_PARKED_TTL_SECS}s awaiting approval");

    let swept = match store::expire_parked_runs(config, &cutoff, &now_str, &error_msg) {
        Ok(swept) => swept,
        Err(e) => {
            tracing::warn!(target: "flows", error = %format_args!("{e:#}"), "[flows] parked-run TTL sweep failed (read continues)");
            return 0;
        }
    };
    for (run_id, flow_id) in &swept {
        if let Err(e) = store::record_run(config, flow_id, "cancelled") {
            tracing::warn!(target: "flows", run_id, flow_id, error = %format_args!("{e:#}"), "[flows] TTL sweep: failed to update flow summary for expired run");
        }
        // Announce the terminal transition (R-m4). `expire_parked_runs` writes
        // the row directly rather than going through `finish_flow_run_row`, so
        // without this the sweep was the one terminal path that emitted no
        // `FlowRunFinished` — the boot sweep already publishes its own. Purely
        // event-driven consumers (the runs rail) would otherwise not observe a
        // TTL-expired run settle until their next poll.
        tracing::debug!(
            target: "flows",
            run_id,
            flow_id,
            "[flows] TTL sweep: publishing FlowRunFinished for expired parked run"
        );
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowRunFinished {
            flow_id: flow_id.to_string(),
            run_id: run_id.to_string(),
            status: "cancelled".to_string(),
        });
        drop_checkpoint(config, run_id).await;
    }
    if !swept.is_empty() {
        tracing::info!(target: "flows", count = swept.len(), ttl_secs = FLOW_PARKED_TTL_SECS, "[flows] parked-run TTL sweep expired stale runs");
    }
    swept.len()
}

/// Boot-time orphan sweep (bug B42, part b): reconciles every `flow_runs` row
/// still at `status = 'running'` that has **no live in-process run** to a
/// terminal `"interrupted"`. A hard crash / SIGKILL / power loss leaves the
/// [`RunRowFinalizer`] drop-guard no chance to run, so a `running` row from the
/// prior process would otherwise stay wedged forever, rendering as a perpetual
/// blank spinner in the run-details sidebar.
///
/// Two independent guards keep the sweep off a run that **this** process owns:
///
/// 1. **A boot floor.** Only rows whose `started_at` predates
///    [`PROCESS_RUN_FLOOR`] are candidates at all, so a row this process
///    inserted is provably out of scope regardless of registration timing —
///    which is what the sweep is actually for: rows left by a *prior* process.
///    Sweeping a live run would not merely mislabel it (its own terminal write
///    would correct that) — it would `drop_checkpoint` it mid-run, and that is
///    unrecoverable.
/// 2. **The in-flight registry.** [`run_registry::is_in_flight`] gates each
///    surviving candidate. Both run entry points now register **before**
///    inserting the row, so within this process a `running` row is never
///    unregistered; this guard covers clock skew and rows stamped by a
///    differently-skewed process.
///
/// The two are deliberately redundant: either alone would be sufficient today,
/// and neither depends on the other's ordering assumption holding.
///
/// Each swept run also updates the flow summary, announces a terminal
/// `FlowRunFinished`, and drops its durable checkpoint (a `running` row is never
/// resumable — only `pending_approval` is). Best-effort by construction: a store
/// error is logged and the sweep returns what it managed.
pub async fn sweep_orphaned_running_runs_on_boot(config: &Config) -> usize {
    match boot_sweep_plan(crate::storage::installed_is_shared()) {
        BootSweepPlan::Nothing => {
            tracing::info!(target: "flows", "[flows] boot sweep: skipped, the storage backend is shared");
            0
        }
        BootSweepPlan::EveryScope => {
            crate::storage::agents::for_each_scope("flows boot sweep", || {
                sweep_orphaned_running_runs_in_scope(config)
            })
            .await
            .into_iter()
            .map(|(_, swept)| swept)
            .sum()
        }
    }
}

/// Which scopes the boot sweep reconciles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootSweepPlan {
    /// `local`, then every agent scope (`crate::storage::agents`).
    EveryScope,
    /// No scope.
    Nothing,
}

/// The boot sweep's scopes. On a backend other processes share (MongoDB) a
/// `running` row below the boot floor — `local` or an agent's — can belong to
/// a run another replica is still driving, and sweeping it would drop that
/// run's checkpoint, so nothing is swept there, as the agent run reaper does.
pub(crate) fn boot_sweep_plan(shared: bool) -> BootSweepPlan {
    if shared {
        BootSweepPlan::Nothing
    } else {
        BootSweepPlan::EveryScope
    }
}

/// [`sweep_orphaned_running_runs_on_boot`] for the current storage scope.
async fn sweep_orphaned_running_runs_in_scope(config: &Config) -> usize {
    let now_str = Utc::now().to_rfc3339();
    const REASON: &str =
        "Run interrupted by an app restart — no live run was executing this row after boot.";

    let floor: &str = PROCESS_RUN_FLOOR.as_str();
    tracing::debug!(target: "flows", floor, "[flows] boot sweep: reconciling only runs started before this process");
    let candidates = match store::list_running_run_ids(config, floor) {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::warn!(target: "flows", error = %format_args!("{e:#}"), "[flows] boot sweep: failed to list running runs (skipping)");
            return 0;
        }
    };
    if candidates.is_empty() {
        return 0;
    }
    tracing::debug!(target: "flows", count = candidates.len(), "[flows] boot sweep: examining running rows for orphans");

    let mut swept = 0usize;
    for (run_id, flow_id) in candidates {
        if run_registry::is_in_flight(&run_id) {
            tracing::debug!(target: "flows", run_id = %run_id, flow_id = %flow_id, "[flows] boot sweep: run is live in-process — leaving it running");
            continue;
        }
        match store::mark_run_interrupted(config, &run_id, &now_str, REASON) {
            Ok(true) => {
                swept += 1;
                if let Err(e) = store::record_run(config, &flow_id, "interrupted") {
                    tracing::warn!(target: "flows", run_id = %run_id, flow_id = %flow_id, error = %format_args!("{e:#}"), "[flows] boot sweep: failed to update flow summary for reconciled run");
                }
                crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowRunFinished {
                    flow_id: flow_id.clone(),
                    run_id: run_id.clone(),
                    status: "interrupted".to_string(),
                });
                drop_checkpoint(config, &run_id).await;
                tracing::info!(target: "flows", run_id = %run_id, flow_id = %flow_id, "[flows] boot sweep: reconciled orphaned running run to 'interrupted'");
            }
            Ok(false) => {
                tracing::debug!(target: "flows", run_id = %run_id, "[flows] boot sweep: row changed status concurrently — skipped");
            }
            Err(e) => {
                tracing::warn!(target: "flows", run_id = %run_id, error = %format_args!("{e:#}"), "[flows] boot sweep: failed to reconcile running run");
            }
        }
    }
    if swept > 0 {
        tracing::info!(target: "flows", count = swept, "[flows] boot sweep reconciled orphaned running runs to 'interrupted'");
    }
    swept
}
