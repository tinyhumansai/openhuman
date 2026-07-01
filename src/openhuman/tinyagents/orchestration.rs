//! Shared orchestration helpers on the `tinyagents` graph layer (issue #4249).
//!
//! openhuman's control plane historically hand-rolled fan-out
//! ([`futures_util::future::join_all`]) and a bespoke detached-sub-agent registry
//! (raw `tokio` `AbortHandle`s, `watch` status channels, tombstone sets). This
//! module is the shared seam that re-expresses that work on `tinyagents`
//! primitives so every orchestration surface (workflow phases, parallel agents,
//! teams, detached sub-agents) routes through one place:
//!
//! - [`run_parallel_fanout`] builds a real [`CompiledGraph`] map step — a
//!   command-routing `dispatch` entry that fans out to N worker nodes running in
//!   the same super-step (`with_parallel` + `with_max_concurrency`), each writing
//!   its result into typed graph state through the reducer, joined at a `collect`
//!   barrier. Results come back in input order regardless of completion order.
//! - The `graph::orchestration` task primitives ([`TaskStore`],
//!   [`OrchestrationTaskKind`], …) are re-exported here so the detached-sub-agent
//!   control plane gets typed task lifecycle bookkeeping (Pending → Running →
//!   Completed/Failed/Cancelled/…) instead of bespoke status enums + watch
//!   channels + tombstones. The store tracks lifecycle; the caller still owns the
//!   executor (the `tokio` task + cooperative cancel + hard abort).
//!
//! Graph lifecycle events are mirrored onto tracing via the shared
//! [`GraphTracingSink`](crate::openhuman::tinyagents::observability::GraphTracingSink).

use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};

use tinyagents::graph::{ClosureStateReducer, Command, GraphBuilder, NodeContext, NodeResult};

use crate::openhuman::tinyagents::observability::GraphTracingSink;

// Re-export the tinyagents task-orchestration primitives so the detached
// sub-agent control plane imports lifecycle types from one openhuman path.
pub use tinyagents::graph::orchestration::{
    InMemoryTaskStore, OrchestrationControlOutcome, OrchestrationTaskFilter, OrchestrationTaskKind,
    OrchestrationTaskRecord, OrchestrationTaskResult, OrchestrationTaskSpec,
    OrchestrationTaskStatus, TaskStore,
};

/// Typed working state for a parallel fan-out: one result slot per worker,
/// filled by the reducer as each worker node completes in any order.
#[derive(Clone)]
struct FanoutState<T: Clone> {
    slots: Vec<Option<T>>,
}

// Manual `Default` (derive would demand `T: Default`, which workers don't have).
impl<T: Clone> Default for FanoutState<T> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

/// Reducer update emitted by a fan-out worker node.
enum FanoutUpdate<T> {
    /// Worker `index` finished; store its result.
    Slot { index: usize, value: Box<T> },
    /// The `collect` fan-in barrier fired; carries no state change.
    Noop,
}

/// Run `run_one(index, item)` for every entry in `items` concurrently on a
/// `tinyagents` fan-out graph, returning the results in **input order**
/// (regardless of completion order). `max_concurrency` bounds how many workers
/// run in the shared super-step.
///
/// This is the generic engine behind the council member fan-out and the
/// `spawn_parallel_agents` tool: pure graph mechanics with no domain knowledge,
/// so it is unit-testable with a trivial closure.
///
/// `label` names the fan-out for tracing. Each `item` is moved into its own
/// worker node (no `Clone` bound on the payload). The worker output `T` must be
/// `Clone` (it rides in the graph's typed state).
pub async fn run_parallel_fanout<I, T, F, Fut>(
    label: &str,
    items: Vec<I>,
    max_concurrency: usize,
    run_one: F,
) -> Result<Vec<T>, String>
where
    I: Send + 'static,
    T: Clone + Send + Sync + 'static,
    F: Fn(usize, I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let n = items.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let worker_ids: Vec<String> = (0..n).map(|i| format!("worker_{i}")).collect();

    let mut builder = GraphBuilder::<FanoutState<T>, FanoutUpdate<T>>::new()
        .with_parallel(true)
        .with_max_concurrency(max_concurrency.max(1))
        .set_reducer(ClosureStateReducer::new(
            |mut s: FanoutState<T>, u: FanoutUpdate<T>| {
                if let FanoutUpdate::Slot { index, value } = u {
                    if let Some(slot) = s.slots.get_mut(index) {
                        *slot = Some(*value);
                    }
                }
                Ok(s)
            },
        ));

    // `dispatch`: command-routing entry that fans out to every worker node.
    let goto_ids = worker_ids.clone();
    builder = builder.add_node("dispatch", move |_s: FanoutState<T>, _c: NodeContext| {
        let goto_ids = goto_ids.clone();
        async move { Ok(NodeResult::Command(Command::default().with_goto(goto_ids))) }
    });

    // One node per worker: runs `run_one(i, item)` and writes the result into
    // its slot. The graph's `NodeHandler` is `Fn` (re-entrant), but each node
    // runs exactly once — hold the moved-in payload in a take-once cell so it is
    // consumed without a `Clone` bound on `I`.
    for (i, item) in items.into_iter().enumerate() {
        let run_one = run_one.clone();
        let node_id = worker_ids[i].clone();
        let cell = Arc::new(StdMutex::new(Some(item)));
        builder = builder.add_node(
            node_id.clone(),
            move |_s: FanoutState<T>, _c: NodeContext| {
                let run_one = run_one.clone();
                let cell = cell.clone();
                async move {
                    let item = cell
                        .lock()
                        .expect("fan-out worker cell poisoned")
                        .take()
                        .expect("fan-out worker node ran more than once");
                    let value = run_one(i, item).await;
                    Ok(NodeResult::Update(FanoutUpdate::Slot {
                        index: i,
                        value: Box::new(value),
                    }))
                }
            },
        );
        builder = builder.add_edge(node_id, "collect");
    }

    // `collect`: fan-in barrier the executor schedules only once every worker
    // edge fired. Leaves accumulated state untouched and finishes the graph.
    builder = builder
        .add_node(
            "collect",
            |_s: FanoutState<T>, _c: NodeContext| async move {
                Ok(NodeResult::Update(FanoutUpdate::Noop))
            },
        )
        .set_entry("dispatch")
        .mark_command_routing("dispatch")
        .set_finish("collect");

    let graph = builder
        .compile()
        .map_err(|e| format!("{label} fan-out graph compile failed: {e}"))?
        .with_event_sink(Arc::new(GraphTracingSink::new(label)));

    tracing::debug!(
        target: "orchestration",
        workers = n,
        max_concurrency = max_concurrency.max(1),
        "[orchestration] running parallel fan-out on tinyagents graph ({label})"
    );

    let execution = graph
        .run(FanoutState {
            slots: vec![None; n],
        })
        .await
        .map_err(|e| format!("{label} fan-out graph run failed: {e}"))?;

    // Every worker node has an edge the executor must traverse, so every slot is
    // populated; a missing slot is a hard invariant break.
    let mut out = Vec::with_capacity(n);
    for (i, slot) in execution.state.slots.into_iter().enumerate() {
        match slot {
            Some(v) => out.push(v),
            None => {
                return Err(format!(
                    "{label} fan-out: worker {i} produced no result (graph invariant broken)"
                ))
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn fanout_runs_every_worker_and_preserves_input_order() {
        let labels = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let ran = Arc::new(AtomicUsize::new(0));
        let ran2 = ran.clone();
        let results = run_parallel_fanout("test", labels, 4, move |i, label| {
            let ran = ran2.clone();
            async move {
                ran.fetch_add(1, Ordering::SeqCst);
                format!("{i}:{label}")
            }
        })
        .await
        .expect("fan-out runs");

        assert_eq!(ran.load(Ordering::SeqCst), 3, "every worker ran once");
        assert_eq!(results, vec!["0:a", "1:b", "2:c"], "results in input order");
    }

    #[tokio::test]
    async fn fanout_empty_is_a_noop() {
        let results =
            run_parallel_fanout::<String, String, _, _>("empty", vec![], 4, |_, _| async move {
                String::new()
            })
            .await
            .expect("empty fan-out runs");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn fanout_handles_single_worker() {
        let results =
            run_parallel_fanout("solo", vec!["x".to_string()], 1, |_, label| async move {
                format!("only:{label}")
            })
            .await
            .expect("single-worker fan-out runs");
        assert_eq!(results, vec!["only:x"]);
    }

    #[tokio::test]
    async fn task_store_tracks_lifecycle() {
        // Smoke the re-exported orchestration primitives: a task moves
        // Pending → Running → Completed and is readable back by id.
        let store = InMemoryTaskStore::new();
        let spec = OrchestrationTaskSpec::new(
            "task-1",
            OrchestrationTaskKind::SubAgent {
                agent: "researcher".to_string(),
            },
        );
        let rec = store.insert(spec).expect("insert");
        assert_eq!(rec.status, OrchestrationTaskStatus::Pending);

        store.mark_running(rec.task_id()).expect("running");
        let done = store
            .complete(rec.task_id(), OrchestrationTaskResult::text("done"))
            .expect("complete");
        assert_eq!(done.status, OrchestrationTaskStatus::Completed);
        assert_eq!(
            store.get(rec.task_id()).map(|r| r.status),
            Some(OrchestrationTaskStatus::Completed)
        );
    }
}
