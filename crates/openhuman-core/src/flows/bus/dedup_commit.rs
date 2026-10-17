//! Commit-on-success half of the `dedup` node's exactly-once contract:
//! [`DedupCommitSubscriber`] settles every `dedup` node's tentative key set
//! when a run finishes, serialized per flow through [`FLOW_COMMIT_LOCKS`].

use crate::config::Config;
use crate::core::events::DomainEvent;
use crate::flows::store;
use crate::flows::tinyflows::state::FlowState;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tinybus::EventHandler;
use tinyflows::model::NodeKind;
use tinyflows::nodes::control_flow::dedup_settle::{self, CommitOutcome, Settlement};

/// Listens for `DomainEvent::FlowRunFinished` and settles every `dedup` node
/// in the finished flow's graph — the host half of the commit-on-success
/// exactly-once contract the tinyflows `dedup` node depends on (issue #5263
/// PR2; the filter half — `DedupNode` — is PR1, already in `vendor/tinyflows`;
/// see `tinyflows::nodes::control_flow::dedup`'s module docs for the full
/// two-sided contract this subscriber implements).
///
/// For every `dedup` node found in the flow's saved graph:
/// - **Success** (`"completed"` / `"completed_with_warnings"`): unions the
///   node's `tentative` key set into its `committed` set, then clears
///   `tentative`. `completed_with_warnings` counts as success — the run
///   reached a terminal, non-retried outcome, so the items it processed are
///   genuinely done even if some non-fatal step warned.
/// - **Anything else** (`"failed"` / `"cancelled"` / `"interrupted"`, or any
///   future/unrecognized status string): clears `tentative` only, leaving
///   `committed` untouched, so the released keys are exactly as unseen as
///   before this run and the flow's next run reprocesses them. An
///   unrecognized status is deliberately treated as failure, not success —
///   "retry an already-done item" is always safe, "silently mark an
///   uncertain outcome as done" is not.
///
/// `StateStore` exposes no prefix-scan, so the only way to know which
/// `dedup:<node_id>:*` keys exist for a flow is to derive `<node_id>` from
/// the flow's own saved graph — this subscriber loads `flow_id`'s graph on
/// every event rather than trying to infer node ids from the event itself.
///
/// Reuses the exact same per-flow `StateStore` namespace
/// (`"flow:<flow_id>"`, see `tinyflows::caps::build_capabilities` in
/// `crates/openhuman-core/src/flows/tinyflows/caps.rs`) the engine's `SqliteStateStore` hands the
/// `dedup` node during the run — that collision with the node's own keys is
/// the entire point.
///
/// Best-effort throughout: every failure here is logged via `tracing::warn!`
/// and swallowed, never propagated — by the time this subscriber observes
/// `FlowRunFinished`, the run has already settled its own `flow_runs` row, so
/// a state-store hiccup here must never retroactively affect run status. A
/// failed commit degrades to "retry next run" (an item is reprocessed, never
/// lost); a failed release degrades to "stays tentative", which the `dedup`
/// node treats as unseen anyway since it only ever consults `committed` —
/// neither failure mode risks silently dropping an item.
///
/// **Commit atomicity (issue #5265, CodeRabbit "Major" on the dedup engine
/// PR):** the per-node commit itself is a read-modify-write
/// (`load(committed) → union(tentative) → store(committed) → delete
/// (tentative)`), not a compare-and-swap. Two overlapping `FlowRunFinished`
/// events for the SAME `flow_id` (e.g. a scheduled run and a manual re-run
/// racing each other) could otherwise interleave their read-modify-writes
/// and have the second writer's `store(committed)` clobber the first
/// writer's union, silently losing that run's committed keys
/// (last-writer-wins). [`handle_finished`](Self::handle_finished) closes
/// that DURABLE half of the race by serializing all of a given flow's
/// dedup-node settlement through a per-`flow_id` lock (see
/// [`FLOW_COMMIT_LOCKS`]) — different flows never contend. This does NOT
/// fix the node-side half: the `dedup` node's own in-run `StateStore`
/// read-modify-write (a single run unioning its own newly-seen items into
/// `tentative`) is a separate, still-open limitation documented on
/// `tinyflows::nodes::control_flow::dedup`'s side; a full CAS-based
/// `StateStore` is deferred.
pub struct DedupCommitSubscriber {
    config: Arc<Config>,
    /// Test-only instrumentation — see [`CommitTestHooks`]. Always `None` in
    /// production (`DedupCommitSubscriber::new`).
    #[cfg(test)]
    test_hooks: Option<Arc<CommitTestHooks>>,
}

/// Process-global registry of per-flow commit locks (issue #5265). Keyed by
/// `flow_id` so unrelated flows never contend with each other; the shared
/// `tokio::sync::Mutex<()>` per key lets [`DedupCommitSubscriber::
/// handle_finished`] hold a guard across its whole (synchronous)
/// read-modify-write section for that flow. Mirrors the same
/// `LazyLock<Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>>` keyed-lock
/// idiom `update_memory_md`'s `WORKSPACE_WRITE_LOCKS` uses for an analogous
/// read-modify-write race (#4458) — grepped for an existing pattern before
/// adding this one; that's the closest match in the crate.
///
/// Deliberately unbounded, matching that precedent: flow ids are bounded in
/// practice (a user's saved flow set), so an evicting map would be
/// complexity this doesn't need yet.
pub(super) static FLOW_COMMIT_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns (creating if needed) the shared async commit lock for `flow_id`.
pub(super) fn flow_commit_lock(flow_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = FLOW_COMMIT_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        map.entry(flow_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Test-only scheduling/witness hooks for proving [`FLOW_COMMIT_LOCKS`]'
/// mutual exclusion. Deliberately **instance-scoped** (owned by one
/// [`DedupCommitSubscriber`], via [`DedupCommitSubscriber::with_test_hooks`])
/// rather than a process-global static: cargo's test harness runs different
/// `#[tokio::test]` functions concurrently on separate OS threads, and a
/// global counter would have unrelated tests' ordinary (unarmed,
/// effectively-instant) commits interleave with — and pollute — a
/// concurrency test's high-water-mark measurement purely by scheduling
/// chance. Scoping the hooks to one test's own `Arc` means only tasks that
/// share that specific subscriber instance can ever touch its counters.
#[cfg(test)]
#[derive(Default)]
pub(super) struct CommitTestHooks {
    pub(super) delay_ms: std::sync::atomic::AtomicU64,
    pub(super) concurrent: std::sync::atomic::AtomicUsize,
    pub(super) max_concurrent: std::sync::atomic::AtomicUsize,
}

impl DedupCommitSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            #[cfg(test)]
            test_hooks: None,
        }
    }

    /// Test constructor: attaches [`CommitTestHooks`] so a test can arm a
    /// delay inside the commit critical section and observe how many
    /// `handle_finished` calls were concurrently inside it.
    #[cfg(test)]
    pub(super) fn with_test_hooks(config: Arc<Config>, hooks: Arc<CommitTestHooks>) -> Self {
        Self {
            config,
            test_hooks: Some(hooks),
        }
    }

    /// No-op unless [`Self::with_test_hooks`] attached hooks — awaited right
    /// after `handle_finished` acquires the per-flow commit lock, while
    /// still holding it. This is what makes it possible to force two
    /// spawned tasks to genuinely interleave on a single-threaded test
    /// executor (there are no other `.await` points inside the
    /// commit/release critical section to give the executor a chance to
    /// poll a contending task) — a test can then prove the lock, not
    /// accidental scheduling luck, is what serializes two overlapping
    /// `FlowRunFinished` events for the same flow. Compiles to an empty
    /// async fn body (zero-cost) in non-test builds.
    async fn maybe_test_delay(&self) {
        #[cfg(test)]
        if let Some(hooks) = &self.test_hooks {
            use std::sync::atomic::Ordering;
            let now = hooks.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            hooks.max_concurrent.fetch_max(now, Ordering::SeqCst);

            let ms = hooks.delay_ms.load(Ordering::SeqCst);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }

            hooks.concurrent.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// The node ids of every `dedup` node in `flow_id`'s saved graph, or an
    /// empty vec (logged, not propagated) if the flow can't be loaded — a
    /// flow deleted between run-finish and this handler firing, or a
    /// transient store error, both degrade to "nothing to settle" rather than
    /// panicking the event bus.
    ///
    /// **Known limitation (issue #5265, Codex "P2" on the dedup engine PR):**
    /// this reads the flow's CURRENT saved definition at settlement time, not
    /// a snapshot of the graph the finishing run actually executed. Nothing
    /// today persists a per-run graph/node-id snapshot — `prepare_flow_run`
    /// loads `Flow` fresh into the spawned run's own task, and that copy is
    /// discarded once the run starts; the `FlowRun` row has no `graph` field.
    /// If a long-running flow is edited (or deleted) while a run is still in
    /// flight:
    /// - a `dedup` node the run wrote `tentative` keys under, then deleted or
    ///   renamed before `FlowRunFinished` fires, is no longer found here — its
    ///   tentative keys are neither committed nor released, so those items
    ///   silently retry on the flow's next run (safe-direction: at worst a
    ///   duplicate, never a lost item, matching this subsystem's existing
    ///   safe-failure posture — see the module doc's "Best-effort throughout"
    ///   paragraph);
    /// - conversely a `dedup` node id newly added to the saved graph after the
    ///   run started is settled here even though the run never executed it
    ///   (a harmless no-op: it has no `tentative` keys to commit/release, see
    ///   `commit`/`release`'s early returns).
    ///
    /// Closing this properly means persisting a per-run graph/dedup-node-id
    /// snapshot at run-start (`start_flow_run_row` or a sibling write) and
    /// having this method read that snapshot instead of `store::get_flow` —
    /// a schema + call-site change bigger than this PR's scope; reported as a
    /// follow-up rather than attempted here.
    pub(super) fn dedup_node_ids(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) -> Vec<String> {
        match store::get_flow(&super::owner::config_for_scope(ctx, &self.config), flow_id) {
            Ok(Some(flow)) => flow
                .graph
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Dedup)
                .map(|n| n.id.clone())
                .collect(),
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, "[dedup-commit] flow no longer exists — skipping");
                Vec::new()
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, error = %e, "[dedup-commit] failed to load flow graph — skipping");
                Vec::new()
            }
        }
    }

    async fn handle_finished(
        &self,
        ctx: crate::core::runtime::DispatchCtx,
        flow_id: &str,
        run_id: &str,
        status: &str,
    ) {
        // As the flow's owner: its own configuration (`super::owner`).
        let config = super::owner::config_for_scope(&ctx, &self.config);
        let node_ids = self.dedup_node_ids(&ctx, flow_id);
        if node_ids.is_empty() {
            tracing::trace!(target: "flows", %flow_id, %run_id, %status, "[dedup-commit] no dedup nodes in this flow — nothing to settle");
            return;
        }

        let success = dedup_settle::is_success_status(status);
        tracing::debug!(
            target: "flows", %flow_id, %run_id, %status, success,
            dedup_node_count = node_ids.len(),
            "[dedup-commit] settling dedup nodes for finished run"
        );

        // Serialize this flow's settlement against any other overlapping
        // `FlowRunFinished` handling for the SAME flow_id — held across the
        // whole read-modify-write loop below so two overlapping runs can
        // never interleave their load(committed)+union(tentative)+
        // store(committed) and lose one run's keys. See `FLOW_COMMIT_LOCKS`
        // docs for the full race this closes.
        let lock = flow_commit_lock(flow_id);
        let lock_guard = lock.lock().await;
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] acquired per-flow commit lock");
        self.maybe_test_delay().await;

        let state = FlowState::open(&config, format!("flow:{flow_id}"));
        for node_id in node_ids {
            self.settle_node(&state, &node_id, success, flow_id, run_id);
        }

        drop(lock_guard);
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] released per-flow commit lock");
    }

    /// Settles one `dedup` node against the flow's state namespace through
    /// `tinyflows`' pure commit/release logic, logging each outcome.
    fn settle_node(
        &self,
        store: &FlowState,
        node_id: &str,
        success: bool,
        flow_id: &str,
        run_id: &str,
    ) {
        match dedup_settle::settle(store, node_id, success) {
            Settlement::Commit(CommitOutcome::NothingTentative) => {
                tracing::trace!(target: "flows", %flow_id, %run_id, node_id, "[dedup-commit] no tentative keys — nothing to commit");
            }
            Settlement::Commit(CommitOutcome::CommitFailed(e)) => tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to write committed set — tentative left in place, will \
                 retry the commit on this node's next successful run"
            ),
            Settlement::Commit(CommitOutcome::Committed {
                added,
                committed_len,
                clear_error,
            }) => {
                tracing::debug!(
                    target: "flows", %flow_id, %run_id, node_id, added, committed_len,
                    "[dedup-commit] committed tentative keys"
                );
                if let Some(e) = clear_error {
                    tracing::warn!(
                        target: "flows", %flow_id, %run_id, node_id, error = %e,
                        "[dedup-commit] committed but failed to clear tentative — harmless: the next \
                         run's dedup load will re-union the same, now-already-committed keys (committed \
                         is a set, so re-adding them is a no-op)"
                    );
                }
            }
            Settlement::Release(Ok(())) => tracing::debug!(
                target: "flows", %flow_id, %run_id, node_id,
                "[dedup-commit] released tentative keys (if any) — will retry next run"
            ),
            Settlement::Release(Err(e)) => tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to release tentative — those keys remain tentative until \
                 a future successful commit reconciles them (harmless: committed stays untouched \
                 either way, so no item is ever wrongly marked done)"
            ),
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for DedupCommitSubscriber {
    fn name(&self) -> &str {
        "flows::dedup_commit"
    }

    fn domains(&self) -> Option<&[&str]> {
        // Same reasoning as `FlowRunDigestSubscriber::domains` just above:
        // `FlowRunFinished` is tagged `"cron"` by `DomainEvent::domain()`.
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::FlowRunFinished {
            flow_id,
            run_id,
            status,
        } = event
        {
            // Settle the run as the agent its flow belongs to (`super::owner`).
            // `FlowRunFinished` carries no tenant: the host's own, and in SaaS
            // none (the run is then not settled here).
            let host = crate::core::runtime::DispatchCtx::host_default();
            super::owner::as_owner(&host, &self.config, flow_id, |ctx| {
                self.handle_finished(ctx, flow_id, run_id, status)
            })
            .await;
        }
    }
}
