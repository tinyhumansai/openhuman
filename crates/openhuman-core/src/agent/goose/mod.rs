//! OpenHuman host adapter for the pinned Goose Agent state machine.
//!
//! This module is deliberately a boundary, not another agent loop. Goose owns
//! step selection and re-entry. OpenHuman owns provider invocation, tools,
//! authorization, progress, cancellation and durable checkpoints.

mod convert;
mod inference;
pub mod qwen;
mod runner;
mod store;
pub mod tools;
mod types;

pub use qwen::{normalize_qwen_response, NormalizedQwenResponse, QwenInvalidCall};
pub use runner::GooseTurnAdapter;
pub use store::{FileGooseCheckpointStore, GooseCheckpointStore, InMemoryGooseCheckpointStore};
pub use tools::{
    is_same_boundary_alternative, is_terminal_route_failure, retain_authorized_alternatives,
    GooseToolRegistry, GooseToolSecurity,
};
pub use types::{
    AcceptedToolAction, GooseCheckpoint, GooseStopReason, GooseTurnOutcome, GooseUsage,
    ToolObservation,
};

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod store_tests;

#[cfg(test)]
#[rustfmt::skip]
mod qwen_tests;
#[cfg(test)]
mod qwen_normalization_tests;
