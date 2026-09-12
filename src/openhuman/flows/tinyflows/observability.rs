//! Host [`tinyflows::observability::RunObserver`] impls for the `flows::`
//! domain.
//!
//! `tinyflows` emits structured run/step records; the host decides what to do
//! with them. Two observers live here:
//!
//! - [`TracingRunObserver`] — log-only, for `run_with_observer` call sites and
//!   as a simple example.
//! - [`FlowRunObserver`] — the real one used by the durable run path (issue
//!   G2, live run observation). It publishes a [`DomainEvent::FlowRunProgress`]
//!   twice per non-trigger node — `running` when the node activates, then its
//!   terminal `success`/`error` when it finishes — so the frontend socket
//!   bridge can stream the run advancing node-by-node. On finish it also
//!   persists a [`FlowRunStep`] incrementally into the run's `flow_runs` row
//!   via `flows::store::upsert_flow_run_step`. All effects are best-effort: a
//!   step that fails to persist is logged, never fatal to the run.
//!
//!   The `running` half matters for more than symmetry: the canvas's
//!   `.flow-node-running` pulse is bound to that status, so while only
//!   `on_step_finish` was implemented the pulse was unreachable and the live
//!   overlay rendered as a completion trail — each node gaining a static ring
//!   only after it had already finished.
//!
//! The durable + journaled run path used by `flows_run`/`flows_resume` now
//! accepts an observer (tinyflows 0.3.1's
//! `run_with_checkpointer_journaled_observed` /
//! `resume_with_checkpointer_journaled_observed`), so [`FlowRunObserver`] sees
//! steps live instead of the run being reconstructed only after it settles.

use std::sync::Arc;

use tinyflows::observability::{ExecutionStep, Run, RunObserver, StepStatus};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use crate::openhuman::flows::{upsert_flow_run_step, FlowRunStep};

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

/// Maps a live [`StepStatus`] to the stable `"success"`/`"error"` string
/// persisted on [`FlowRunStep::status`] and published on
/// [`DomainEvent::FlowRunProgress`]. `StepStatus` does not derive `PartialEq`,
/// so this matches rather than compares.
fn step_status_str(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Success => "success",
        StepStatus::Error => "error",
    }
}

/// The durable-run observer (issue G2): persists each finished step
/// incrementally and streams a progress event to the frontend.
///
/// Held by the engine as `Arc<dyn RunObserver>` and cloned into node handlers
/// that run across threads, so it must be cheap and non-blocking-ish — it does
/// one small SQLite read-modify-write per step (WAL + `busy_timeout` absorb the
/// concurrent settle write) and one fire-and-forget broadcast publish. Never
/// logs step output/PII: only ids, status, and timing are logged/emitted.
pub struct FlowRunObserver {
    config: Arc<Config>,
    flow_id: String,
    /// The run's checkpointer thread id, which doubles as the `flow_runs` row
    /// id and the `run_id` on the emitted progress event.
    run_id: String,
}

impl FlowRunObserver {
    /// Builds an observer bound to one `flows_run` / `flows_resume` invocation.
    pub fn new(config: Arc<Config>, flow_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            config,
            flow_id: flow_id.into(),
            run_id: run_id.into(),
        }
    }
}

impl RunObserver for FlowRunObserver {
    fn on_run_start(&self, run_id: &str) {
        tracing::debug!(
            target: "flows",
            flow_id = %self.flow_id,
            run_id = %self.run_id,
            engine_run_id = %run_id,
            "[flows] observer: run start"
        );
    }

    /// Announces a node as `running` the moment it activates, so the canvas can
    /// show what is executing *now* rather than only what has already finished.
    ///
    /// Without this the overlay could never do that: `on_step_finish` is the
    /// only event that ever fired, so the socket only ever carried `success` /
    /// `failed`, and a node stayed unstyled until it was already done. The
    /// canvas's `.flow-node-running` pulse (`flow-node-run-pulse`, written for
    /// exactly this) was therefore unreachable, and the "live run overlay" read
    /// as a completion trail — nodes acquiring a static ring after the fact.
    ///
    /// Publish-only, deliberately: the durable `flow_runs` row stays
    /// finish-driven. A started-but-unfinished node has no output, duration, or
    /// terminal status to record, and writing a placeholder row would put a
    /// `running` step into run history that the post-hoc reconstruction at
    /// settle would then have to reconcile away. The socket event is ephemeral
    /// UI state; the row remains the source of truth.
    fn on_step_start(&self, node_id: &str) {
        tracing::debug!(
            target: "flows",
            flow_id = %self.flow_id,
            run_id = %self.run_id,
            node = %node_id,
            "[flows] observer: step started — publishing FlowRunProgress(running)"
        );
        BUS.publish(DomainEvent::FlowRunProgress {
            run_id: self.run_id.clone(),
            node_id: node_id.to_string(),
            status: "running".to_string(),
        });
    }

    fn on_step_finish(&self, step: &ExecutionStep) {
        let status = step_status_str(&step.status);
        tracing::debug!(
            target: "flows",
            flow_id = %self.flow_id,
            run_id = %self.run_id,
            node = %step.node_id,
            status,
            duration_ms = step.duration_ms,
            "[flows] observer: step finished — persisting incrementally"
        );

        // Persist the live step. `duration_ms` is `u128` on the engine side;
        // clamp into `u64` (a node executor taking > 584 million years is not a
        // real concern, but never panic on cast).
        if !step.diagnostics.is_empty() {
            tracing::warn!(
                target: "flows",
                flow_id = %self.flow_id,
                run_id = %self.run_id,
                node = %step.node_id,
                diagnostics = ?step.diagnostics,
                "[flows] observer: step reported null-resolved expression(s) — persisting for the run view"
            );
        }
        let flow_step = FlowRunStep {
            node_id: step.node_id.clone(),
            output: step.output.clone(),
            port: None,
            status: Some(status.to_string()),
            duration_ms: Some(u64::try_from(step.duration_ms).unwrap_or(u64::MAX)),
            diagnostics: step
                .diagnostics
                .iter()
                .map(|d| serde_json::to_value(d).unwrap_or(serde_json::Value::Null))
                .collect(),
        };
        if let Err(e) = upsert_flow_run_step(&self.config, &self.run_id, &flow_step) {
            tracing::warn!(
                target: "flows",
                flow_id = %self.flow_id,
                run_id = %self.run_id,
                node = %step.node_id,
                error = %e,
                "[flows] observer: failed to persist incremental step (run continues; post-hoc reconstruction will fill it in at settle)"
            );
        }

        // Best-effort progress feed to the frontend socket bridge. The durable
        // `flow_runs` row remains the source of truth; this event may be
        // dropped under broadcast lag, so the UI keeps its 2s poller fallback.
        tracing::debug!(
            target: "flows",
            flow_id = %self.flow_id,
            run_id = %self.run_id,
            node = %step.node_id,
            status,
            "[flows] observer: publishing FlowRunProgress"
        );
        BUS.publish(DomainEvent::FlowRunProgress {
            run_id: self.run_id.clone(),
            node_id: step.node_id.clone(),
            status: status.to_string(),
        });
    }

    fn on_run_finish(&self, run: &Run) {
        tracing::debug!(
            target: "flows",
            flow_id = %self.flow_id,
            run_id = %self.run_id,
            engine_run_id = %run.id,
            steps = run.steps.len(),
            status = ?run.status,
            "[flows] observer: run finished"
        );
    }
}

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
