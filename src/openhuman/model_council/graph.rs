//! Parallel council fan-out expressed on the `tinyagents` graph layer (#4249, #27).
//!
//! This is the `model_council` folder's `graph.rs` per the per-folder graph
//! convention: the folder's tinyagents fan-out graph definition lives here;
//! `council.rs` drives it.
//!
//! The council runs N member models concurrently, then a chair synthesizes their
//! answers. Historically the fan-out was a hand-rolled
//! [`futures_util::future::join_all`]; this module re-expresses the *map* half as
//! a real `tinyagents` [`CompiledGraph`]:
//!
//! ```text
//!            ┌─ member_0 ─┐
//!   dispatch ┼─ member_1 ─┼─ collect (finish)
//!            └─ member_n ─┘
//! ```
//!
//! `dispatch` is a command-routing node that fans out to every member via
//! [`Command::with_goto`]; the members run in the same superstep (`with_parallel`)
//! and each writes its [`CouncilMemberResult`] into the graph state through the
//! reducer; `collect` is the fan-in barrier that the executor only schedules once
//! every member edge has fired. The chair synthesis stays outside the graph (it
//! is a single sequential call), matching the previous control flow.
//!
//! The member runner is injected ([`run_member_fanout`]) so the graph mechanics
//! (typed state + reducer + parallel super-steps + fan-in barrier) are unit
//! tested with a trivial closure, while production passes the real provider call.
//! This is the first openhuman feature driven on the SDK's StateGraph primitives.

use std::future::Future;
use std::sync::Arc;

use tinyagents::graph::ClosureStateReducer;
use tinyagents::graph::{Command, GraphBuilder, NodeContext, NodeResult};

use crate::openhuman::config::Config;
use crate::openhuman::tinyagents::observability::GraphTracingSink;

use super::council::{run_member_answer_inner, CouncilMemberResult};

/// Typed working state threaded through the council graph: one slot per member,
/// filled in by the reducer as each member node completes (in any order).
#[derive(Clone, Default)]
struct CouncilState {
    members: Vec<Option<CouncilMemberResult>>,
}

/// Reducer updates emitted by the graph nodes.
enum CouncilUpdate {
    /// A member seat finished; store its result at `index`.
    Member {
        index: usize,
        result: Box<CouncilMemberResult>,
    },
    /// The fan-in `collect` node fired; it carries no state change.
    Noop,
}

/// Run the council member fan-out on the tinyagents graph and return the member
/// results in seat order. The chair synthesis is performed by the caller.
///
/// `models` is the already-normalized member model list; `config`/`question` are
/// cloned into the node closures (the graph requires `'static` handlers).
pub async fn run_council_members_via_graph(
    config: Arc<Config>,
    question: Arc<str>,
    models: Vec<String>,
    temperature: Option<f64>,
) -> Result<Vec<CouncilMemberResult>, String> {
    run_member_fanout(models, move |model| {
        let config = config.clone();
        let question = question.clone();
        async move { run_member_answer_inner(&config, &question, &model, temperature).await }
    })
    .await
}

/// Build and run the parallel member fan-out graph, invoking `run_one(model)` for
/// each seat. Pure graph mechanics — no provider knowledge — so it is unit
/// testable with a mock runner.
async fn run_member_fanout<F, Fut>(
    models: Vec<String>,
    run_one: F,
) -> Result<Vec<CouncilMemberResult>, String>
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = CouncilMemberResult> + Send + 'static,
{
    let n = models.len();
    let member_ids: Vec<String> = (0..n).map(|i| format!("member_{i}")).collect();

    let mut builder = GraphBuilder::<CouncilState, CouncilUpdate>::new()
        .with_parallel(true)
        .with_max_concurrency(n.max(1))
        .set_reducer(ClosureStateReducer::new(
            |mut s: CouncilState, u: CouncilUpdate| {
                if let CouncilUpdate::Member { index, result } = u {
                    if let Some(slot) = s.members.get_mut(index) {
                        *slot = Some(*result);
                    }
                }
                Ok(s)
            },
        ));

    // `dispatch`: command-routing entry that fans out to every member seat.
    let goto_ids = member_ids.clone();
    builder = builder.add_node("dispatch", move |_s: CouncilState, _c: NodeContext| {
        let goto_ids = goto_ids.clone();
        async move { Ok(NodeResult::Command(Command::default().with_goto(goto_ids))) }
    });

    // One node per member seat: runs the member answer and writes it back.
    for (i, model) in models.into_iter().enumerate() {
        let run_one = run_one.clone();
        let node_id = member_ids[i].clone();
        builder = builder.add_node(node_id.clone(), move |_s: CouncilState, _c: NodeContext| {
            let run_one = run_one.clone();
            let model = model.clone();
            async move {
                let result = run_one(model).await;
                Ok(NodeResult::Update(CouncilUpdate::Member {
                    index: i,
                    result: Box::new(result),
                }))
            }
        });
        // Every seat fans into the `collect` barrier.
        builder = builder.add_edge(node_id, "collect");
    }

    // `collect`: fan-in barrier the executor only runs once every member edge
    // fired. It leaves the accumulated state untouched and finishes the graph.
    builder = builder
        .add_node("collect", |_s: CouncilState, _c: NodeContext| async move {
            Ok(NodeResult::Update(CouncilUpdate::Noop))
        })
        .set_entry("dispatch")
        .mark_command_routing("dispatch")
        .set_finish("collect");

    let graph = builder
        .compile()
        .map_err(|e| format!("council graph compile failed: {e}"))?
        // Mirror the executor's node/run lifecycle onto tracing (#28 observability).
        .with_event_sink(Arc::new(GraphTracingSink::new("council:graph")));

    tracing::debug!(
        members = n,
        "[model-council] running member fan-out on tinyagents graph"
    );
    let execution = graph
        .run(CouncilState {
            members: vec![None; n],
        })
        .await
        .map_err(|e| format!("council graph run failed: {e}"))?;

    // Every member node ran (each seat has an edge the executor must traverse),
    // so every slot is populated; fall back defensively to a failure result.
    let results = execution
        .state
        .members
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            slot.unwrap_or_else(|| {
                tracing::warn!(
                    seat = i,
                    "[model-council] member slot empty after graph run"
                );
                CouncilMemberResult {
                    model: "unknown".to_string(),
                    response: None,
                    error: Some("member seat produced no result on the graph path".to_string()),
                }
            })
        })
        .collect();
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The fan-out preserves seat order regardless of completion order, runs every
    /// seat, and merges each result into the right slot via the reducer.
    #[tokio::test]
    async fn fanout_runs_every_seat_and_preserves_order() {
        let models = vec!["m-a".to_string(), "m-b".to_string(), "m-c".to_string()];
        let ran = Arc::new(AtomicUsize::new(0));
        let ran2 = ran.clone();
        let results = run_member_fanout(models, move |model| {
            let ran = ran2.clone();
            async move {
                ran.fetch_add(1, Ordering::SeqCst);
                CouncilMemberResult {
                    model: model.clone(),
                    response: Some(format!("answer from {model}")),
                    error: None,
                }
            }
        })
        .await
        .expect("graph fan-out runs");

        assert_eq!(
            ran.load(Ordering::SeqCst),
            3,
            "every seat node executed once"
        );
        assert_eq!(results.len(), 3, "one result per seat");
        // Results are returned in seat (input) order, not completion order.
        assert_eq!(results[0].model, "m-a");
        assert_eq!(results[1].model, "m-b");
        assert_eq!(results[2].model, "m-c");
        assert_eq!(
            results[2].response.as_deref(),
            Some("answer from m-c"),
            "each seat's result lands in its own slot"
        );
    }

    /// The observability sink receives the executor's lifecycle events for every
    /// node in the fan-out (run + node start/complete across the supersteps).
    #[tokio::test]
    async fn tracing_sink_receives_graph_lifecycle_events() {
        let sink = GraphTracingSink::new("test:graph");
        let counter = sink.counter();

        let graph = GraphBuilder::<CouncilState, CouncilUpdate>::new()
            .set_reducer(ClosureStateReducer::new(
                |s: CouncilState, _u: CouncilUpdate| Ok(s),
            ))
            .add_node("solo", |_s: CouncilState, _c: NodeContext| async move {
                Ok(NodeResult::Update(CouncilUpdate::Noop))
            })
            .set_entry("solo")
            .set_finish("solo")
            .compile()
            .expect("compiles")
            .with_event_sink(Arc::new(sink));

        graph
            .run(CouncilState::default())
            .await
            .expect("graph runs");

        // At minimum: RunStarted + NodeStarted + NodeCompleted + RunCompleted.
        assert!(
            counter.load(Ordering::Relaxed) >= 4,
            "sink should observe the run+node lifecycle events, saw {}",
            counter.load(Ordering::Relaxed)
        );
    }

    /// A single-member council still builds a valid graph (one branch + barrier).
    #[tokio::test]
    async fn fanout_handles_single_member() {
        let results = run_member_fanout(vec!["solo".to_string()], move |model| async move {
            CouncilMemberResult {
                model,
                response: Some("only answer".to_string()),
                error: None,
            }
        })
        .await
        .expect("single-member graph runs");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].response.as_deref(), Some("only answer"));
    }
}
