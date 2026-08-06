//! Durable run ledger for agent and workflow execution state.
//!
//! This submodule extends `session_db` with a queryable, restart-survivable
//! ledger for background agent/workflow runs. Conversation transcripts remain
//! in the thread/session stores; this ledger stores compact run metadata,
//! child lineage, events, telemetry, and checkpoint references.

pub mod ops;
pub mod store;
pub mod types;

pub use ops::{
    append_run_event, claim_agent_team_task, complete_agent_team_task, get_agent_run,
    get_agent_team, get_agent_team_member, get_agent_team_task, get_workflow_run,
    interrupt_orphaned_agent_runs, list_agent_runs, list_agent_team_members, list_agent_team_tasks,
    list_agent_teams, list_recent_run_events, list_workflow_runs, mark_agent_team_member_idle,
    mark_agent_team_member_running, release_agent_team_task, shutdown_agent_team_member,
    transition_agent_run_status, upsert_agent_run, upsert_agent_team, upsert_agent_team_member,
    upsert_agent_team_task, upsert_run_telemetry, upsert_workflow_run,
};
pub use types::{
    AgentRun, AgentRunKind, AgentRunListRequest, AgentRunListResponse, AgentRunStatus,
    AgentRunUpsert, AgentTeam, AgentTeamListRequest, AgentTeamListResponse, AgentTeamMember,
    AgentTeamMemberStatus, AgentTeamMemberUpsert, AgentTeamStatus, AgentTeamTask,
    AgentTeamTaskStatus, AgentTeamTaskUpsert, AgentTeamUpsert, ClaimOutcome, CompletionOutcome,
    RunEvent, RunEventAppend, RunEventListRequest, RunEventListResponse, RunTelemetry,
    RunTelemetryUpsert, WorkflowRun, WorkflowRunListRequest, WorkflowRunListResponse,
    WorkflowRunStatus, WorkflowRunUpsert,
};
