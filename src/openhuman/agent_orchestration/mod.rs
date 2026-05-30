//! High-level agent-to-agent orchestration domain.
//!
//! This module owns the control-plane semantics for coordinating multiple
//! agent workers from one parent session. The lower-level
//! [`crate::openhuman::agent::harness`] module remains responsible for prompt
//! construction, tool filtering, and the actual sub-agent run loop.

mod ops;
pub mod tools;
pub mod types;

#[cfg(test)]
mod ops_tests;

pub use ops::{AgentOrchestrationSession, OrchestrationError};
pub use types::{
    AgentMessage, AgentOrchestrationEvent, AgentSnapshot, AgentStatus, CloseAgentRequest,
    FollowUpRequest, MessageAgentRequest, ResumeAgentRequest, SpawnAgentRequest,
    SpawnAgentResponse, WaitAgentOptions, WaitAgentResponse,
};
