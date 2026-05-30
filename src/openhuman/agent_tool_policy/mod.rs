//! Agent tool-policy profiling and enforcement.
//!
//! This domain keeps prompt-visible tools and runtime execution aligned with
//! the session's configured channel permission boundary.

mod engine;
mod prompt;
mod types;

pub use engine::ToolPolicyEngine;
pub use prompt::render_tool_policy_boundary;
pub use types::{
    TaskProfile, TaskRiskLevel, ToolCapability, ToolPolicyAction, ToolPolicyDecision,
    ToolPolicySession,
};
