//! Parallel council fan-out expressed on the shared TinyAgents map/reduce seam.
//!
//! The council runs N member models concurrently, then a chair synthesizes their
//! answers. Historically the fan-out was a hand-rolled `join_all`, then a local
//! dispatch/worker/collect graph. This module now routes the map half directly
//! through `tinyagents::graph::parallel::map_reduce`. The chair synthesis stays
//! outside the map step because it is a single sequential call.

use std::future::Future;
use std::sync::Arc;
use tinyagents::graph::parallel::{map_reduce, FailurePolicy, ParallelOptions};

use crate::openhuman::config::Config;

use super::council::{run_member_answer_inner, CouncilMemberResult};

/// Run the council member fan-out and return member results in seat order.
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

/// Build and run the parallel member fan-out, invoking `run_one(model)` for
/// each seat. Pure fan-out mechanics, so tests can pass a mock runner.
async fn run_member_fanout<F, Fut>(
    models: Vec<String>,
    run_one: F,
) -> Result<Vec<CouncilMemberResult>, String>
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = CouncilMemberResult> + Send + 'static,
{
    let n = models.len();
    tracing::debug!(
        members = n,
        "[model-council] running member fan-out on tinyagents map_reduce"
    );
    let options = ParallelOptions::default()
        .with_max_concurrency(n.max(1))
        .with_failure_policy(FailurePolicy::CollectAll);
    let outcome = map_reduce(models, options, move |_i, model| {
        let run_one = run_one.clone();
        async move { Ok(run_one(model).await) }
    })
    .await
    .map_err(|e| format!("council fan-out failed: {e}"))?;

    let mut results = Vec::with_capacity(n);
    for item in outcome.outcomes {
        match item.result {
            Ok(value) => results.push(value),
            Err(err) => {
                return Err(format!(
                    "council fan-out: worker {} failed: {err}",
                    item.index
                ));
            }
        }
    }
    if results.len() != n {
        return Err(format!(
            "council fan-out: expected {n} result(s), got {}",
            results.len()
        ));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        .expect("fan-out runs");

        assert_eq!(ran.load(Ordering::SeqCst), 3, "every seat ran once");
        assert_eq!(results.len(), 3, "one result per seat");
        assert_eq!(results[0].model, "m-a");
        assert_eq!(results[1].model, "m-b");
        assert_eq!(results[2].model, "m-c");
        assert_eq!(
            results[2].response.as_deref(),
            Some("answer from m-c"),
            "each seat's result lands in its own slot"
        );
    }

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
        .expect("single-member fan-out runs");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].response.as_deref(), Some("only answer"));
    }
}
