//! High-level agent-to-agent orchestration domain.
//!
//! This module owns the control-plane semantics for coordinating multiple
//! agent workers from one parent session. The lower-level
//! [`crate::openhuman::agent::harness`] module remains responsible for prompt
//! construction, tool filtering, and the actual sub-agent run loop.
//!
//! Execution fans out on TinyAgents **graphs**: [`workflow_runs`] schedules
//! phase DAGs on a graph engine, [`agent_teams`] runs members through a
//! conditional-routing graph, [`delegation`] wires the durable
//! plan→execute⇄review→finalize graph, and parallel fanout goes through
//! `tinyagents::graph::parallel::map_reduce`. What stays here is the product
//! layer: durable SQL/JSON run ledgers, validation, cancellation semantics,
//! compatibility events, and JSON-RPC/tool response formatting.
//! [`running_subagents`] mirrors detached-sub-agent lifecycle into a
//! TinyAgents task store; porting more of that lifecycle upstream is tracked
//! in WP-5 of `docs/tinyagents-migration-plan-2026-07-22.md`.

pub mod agent_teams;
pub(crate) mod background_completions;
pub(crate) mod background_delivery;
pub mod command_center;
pub(crate) mod delegation;
pub mod harness_audit;
mod ops;
pub mod pairing;
mod pairing_schemas;
pub(crate) mod parent_context;
pub(crate) mod run_ledger_finalize;
pub(crate) mod running_subagents;
pub(crate) mod spawn_parallel_graph;
pub mod subagent_control;
pub(crate) mod subagent_events;
pub(crate) mod subagent_sessions;
pub mod tools;
pub mod types;
pub mod workflow_runs;
pub mod worktree;
mod worktree_schemas;

#[cfg(test)]
mod ops_tests;

pub use agent_teams::{all_agent_team_controller_schemas, all_agent_team_registered_controllers};
pub use command_center::{
    all_command_center_controller_schemas, all_command_center_registered_controllers,
};
pub use ops::{AgentOrchestrationSession, OrchestrationError};
pub use pairing_schemas::{
    all_controller_schemas as all_pairing_controller_schemas,
    all_registered_controllers as all_pairing_registered_controllers,
};
pub use subagent_control::{
    all_controller_schemas as all_subagent_control_controller_schemas,
    all_registered_controllers as all_subagent_control_registered_controllers,
};
pub use types::{
    AgentMessage, AgentOrchestrationEvent, AgentSnapshot, AgentStatus, CloseAgentRequest,
    FollowUpRequest, MessageAgentRequest, ResumeAgentRequest, SpawnAgentRequest,
    SpawnAgentResponse, WaitAgentOptions, WaitAgentResponse,
};
pub use workflow_runs::{
    all_workflow_run_controller_schemas, all_workflow_run_registered_controllers,
};
pub use worktree::{BaseRef, GitWorktreeIsolation, WorktreeError, WorktreeStatus};
pub use worktree_schemas::{
    all_controller_schemas as all_worktree_controller_schemas,
    all_registered_controllers as all_worktree_registered_controllers,
};
