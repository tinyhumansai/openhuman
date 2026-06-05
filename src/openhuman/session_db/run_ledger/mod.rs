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
    append_run_event, get_agent_run, list_agent_runs, list_recent_run_events, upsert_agent_run,
    upsert_run_telemetry, upsert_workflow_run,
};
pub use types::{
    AgentRun, AgentRunKind, AgentRunListRequest, AgentRunListResponse, AgentRunStatus,
    AgentRunUpsert, RunEvent, RunEventAppend, RunEventListRequest, RunEventListResponse,
    RunTelemetry, RunTelemetryUpsert, WorkflowRun, WorkflowRunStatus, WorkflowRunUpsert,
};
