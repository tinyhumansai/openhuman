//! Log-only [`tinyflows::observability::RunObserver`] for the `flows::`
//! domain.
//!
//! `tinyflows` emits structured run/step records; the host decides what to do
//! with them. B1 only logs — persisting steps/runs for a run-history view is
//! B2+ (see `my_docs/ohxtf/b1-engine-seam-domain/07-execution-and-hitl.md`).
//! Note the durable path (`engine::run_with_checkpointer`, what `flows_run`
//! uses) installs a `NoopObserver` internally in 0.2, so this observer is
//! wired for `run_with_observer` call sites, not the durable run path itself.

use tinyflows::observability::{ExecutionStep, Run, RunObserver};

/// Logs run/step lifecycle events with grep-friendly `[flows]` prefixes.
pub struct TracingRunObserver {
    pub run_label: String,
}

impl RunObserver for TracingRunObserver {
    fn on_run_start(&self, run_id: &str) {
        tracing::info!(target: "flows", run_label = %self.run_label, %run_id, "[flows] run start");
    }

    fn on_step_finish(&self, step: &ExecutionStep) {
        tracing::debug!(
            target: "flows",
            run_label = %self.run_label,
            node = %step.node_id,
            status = ?step.status,
            duration_ms = step.duration_ms,
            "[flows] step finished"
        );
    }

    fn on_run_finish(&self, run: &Run) {
        tracing::info!(
            target: "flows",
            run_label = %self.run_label,
            id = %run.id,
            status = ?run.status,
            steps = run.steps.len(),
            "[flows] run finish"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tinyflows::observability::{RunStatus, StepStatus};

    #[test]
    fn callbacks_do_not_panic() {
        let observer = TracingRunObserver {
            run_label: "test".to_string(),
        };
        observer.on_run_start("run-1");
        observer.on_step_finish(&ExecutionStep {
            node_id: "n".to_string(),
            status: StepStatus::Success,
            output: Value::Null,
            duration_ms: 5,
        });
        observer.on_run_finish(&Run {
            id: "run-1".to_string(),
            status: RunStatus::Completed,
            steps: Vec::new(),
        });
    }
}
