//! Host-side execution pipeline for `spawn_parallel_agents`.
//!
//! The tool wrapper still owns `ToolResult` translation. This module owns
//! tool request parsing, parent-context validation,
//! worktree preflight, progress/event projection, worker fanout, final JSON
//! formatting, and the topology surface.
//!
//! **Write safety.** Whether a worker *needs* a claim on the shared workspace is
//! an OpenHuman decision — it reads sandbox mode, tool permissions and the
//! isolation request. Whether the claims of a whole batch can be granted
//! together is not, and goes through
//! [`plan_shared_workspace_dispatch`](tinyagents_graph::parallel::plan_shared_workspace_dispatch).
//! The rejection sentences stay here, which is why the crate reports conflicts
//! as data.
//!
//! ## Module layout
//!
//! - [`types`] — shared worker/result/lineage types.
//! - [`staging`] — OpenHuman policy admission (an `AdmissionPolicy` over
//!   `tinyagents_orchestration::parallel::Plan`, which owns arbitration).
//! - [`dispatch`] — worktree preflight and progress projection for the
//!   `dispatch` phase, turning admitted tasks into staged workers.
//! - [`workers`] — single-worker execution; ordering, the serial fallback and
//!   bounded fan-out are `tinyagents_orchestration::parallel::run`.
//! - [`collect`] — collecting fanned-out results into the tool's final shape.
//! - [`run`] — the public entry points that tie the above together.

mod collect;
mod dispatch;
mod run;
mod staging;
mod types;
mod workers;

pub(crate) use collect::{format_spawn_parallel_success, SpawnParallelGraphOutcome};
pub(crate) use run::run_spawn_parallel_tasks_with_cancellation_and_workspace;
pub(crate) use types::ParallelAgentTask;

#[cfg(test)]
pub(crate) use staging::{
    prepare_spawn_parallel_tasks_from_defs, ParallelTaskRejectionKind, SpawnParallelTaskPreflight,
};
#[cfg(test)]
pub(crate) use types::{ParallelAgentLineage, ParallelAgentResult, ParallelAgentStatus};
