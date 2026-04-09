pub mod engine;
pub mod executor;
pub mod global;
pub mod prompt;
mod schemas;
pub mod situation_report;
pub mod store;
pub mod types;

// Keep decision_log for potential future dedup queries against the log table.
pub mod decision_log;

#[cfg(test)]
mod integration_test;

pub use engine::SubconsciousEngine;
pub use schemas::{
    all_controller_schemas as all_subconscious_controller_schemas,
    all_registered_controllers as all_subconscious_registered_controllers,
};
pub use types::{
    Escalation, EscalationStatus, SubconsciousLogEntry, SubconsciousStatus, SubconsciousTask,
    TaskRecurrence, TaskSource, TickDecision, TickResult,
};
