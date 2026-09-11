//! The `tinyflows` capability seam: wires the `tinyflows` workflow engine
//! (an external, host-agnostic crate — validate → compile → run on its own
//! in-crate state-graph runtime) to real OpenHuman services.
//!
//! This module is export-focused. Six of the seven capability adapters plus
//! the two run entry points — [`build_capabilities`] and
//! [`open_flow_checkpointer`], re-exported below — live in [`caps`]; the
//! `memory` node's `MemoryProvider` adapter (`OpenHumanMemory`) lives in its
//! own [`memory_adapter`] module, per this repo's ~500-line file-size
//! convention (`caps.rs` is already large); the durable SQLite checkpoint
//! backend lives in `tinyflows_sqlite::checkpoint`; run observability logging lives
//! in [`observability`]; post-run Langfuse export of a run's durable graph
//! observations lives in [`langfuse_export`]. The `flows::` domain
//! (`src/openhuman/flows/ops.rs`) calls [`build_capabilities`] /
//! [`open_flow_checkpointer`] to drive a run and
//! [`langfuse_export::export_flow_run_trace`] after it settles.

pub mod caps;
#[cfg(test)]
#[path = "checkpoint_compat_tests.rs"]
mod checkpoint_compat_tests;
pub mod langfuse_export;
pub mod memory_adapter;
/// End-to-end coverage for the `memory` node through the REAL engine + real
/// `OpenHumanMemory` adapter + real store — see the module doc there for why
/// this lives apart from `tests.rs`'s general capability-seam smoke tests.
#[cfg(test)]
mod memory_node_e2e_tests;
pub mod observability;
#[cfg(test)]
#[path = "tinyflows_tests.rs"]
mod tests;

/// The durable SQLite checkpoint store. It lives in `tinyflows-sqlite` — the
/// schema and the on-disk format are shared with every host that resumes an
/// interrupted run — and is re-exported under its historical name so existing
/// `tinyflows::checkpoint_sqlite::…` paths keep resolving.
pub use tinyflows_sqlite::checkpoint as checkpoint_sqlite;

pub use caps::{build_capabilities, open_flow_checkpointer};
pub use memory_adapter::OpenHumanMemory;
