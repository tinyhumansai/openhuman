//! The `tinyflows` capability seam: wires the `tinyflows` workflow engine
//! (an external, host-agnostic crate — validate → compile → run on
//! `tinyagents`) to real OpenHuman services.
//!
//! This module is export-focused. The five capability adapters plus the two
//! run entry points — [`build_capabilities`] and [`open_flow_checkpointer`],
//! re-exported below — live in [`caps`]; run observability logging lives in
//! [`observability`]. The `flows::` domain (`src/openhuman/flows/ops.rs`) calls
//! [`build_capabilities`] / [`open_flow_checkpointer`] to drive a run.

pub mod caps;
pub mod observability;
#[cfg(test)]
mod tests;

pub use caps::{build_capabilities, open_flow_checkpointer};
