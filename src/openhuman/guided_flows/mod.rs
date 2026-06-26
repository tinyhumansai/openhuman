//! Guided recommendation flows domain.
//!
//! Provides a reusable state-machine engine for quiz-style or conversational
//! intake flows that guide users to recommendations, decisions, or next actions.
//!
//! Architecture: flow definitions → engine (state machine) → recommendation generation.
//! All business logic lives in Rust; the app layer only renders prompts and collects answers.
//!
//! Log prefix: `[guided_flows]`

pub mod engine;
mod rpc;
mod schemas;
pub mod scoring;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_guided_flows_controller_schemas,
    all_registered_controllers as all_guided_flows_registered_controllers,
    schemas as guided_flows_schemas,
};
pub use types::{FlowDefinition, FlowSession, FlowSessionState, Recommendation};
