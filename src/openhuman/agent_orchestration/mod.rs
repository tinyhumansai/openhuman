//! High-level agent-to-agent orchestration domain.
//!
//! This module owns the control-plane semantics for coordinating multiple
//! agent workers from one parent session. The lower-level
//! [`crate::openhuman::agent::harness`] module remains responsible for prompt
//! construction, tool filtering, and the actual sub-agent run loop.

pub mod agent_teams;
pub mod command_center;
mod ops;
pub mod tools;
pub mod types;
pub mod workflow_runs;

#[cfg(test)]
mod ops_tests;

pub use agent_teams::{all_agent_team_controller_schemas, all_agent_team_registered_controllers};
pub use command_center::{
    all_command_center_controller_schemas, all_command_center_registered_controllers,
};
pub use ops::{AgentOrchestrationSession, OrchestrationError};
pub use types::{
    AgentMessage, AgentOrchestrationEvent, AgentSnapshot, AgentStatus, CloseAgentRequest,
    FollowUpRequest, MessageAgentRequest, ResumeAgentRequest, SpawnAgentRequest,
    SpawnAgentResponse, WaitAgentOptions, WaitAgentResponse,
};
pub use workflow_runs::{
    all_workflow_run_controller_schemas, all_workflow_run_registered_controllers,
};
