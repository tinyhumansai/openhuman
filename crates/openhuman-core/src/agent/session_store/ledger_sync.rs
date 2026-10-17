//! Synchronous, workspace-keyed views of the calling tenant's run ledger.
//!
//! The run ledger itself is TinyAgents' async [`RunLedgerStore`], reached for a
//! tenant through [`require_ledger`](super::require_ledger). Hosts and tests
//! that hold a workspace directory and want one record in or out (seeding a
//! run, reading it back) use these: each opens the tenant's ledger for
//! `workspace_dir` and runs one port call on the shared storage bridge
//! ([`with_ledger`]).

use std::path::Path;

use tinyagents_session::run_ledger::{
    AgentRun, AgentRunUpsert, AgentTeam, AgentTeamMember, AgentTeamMemberUpsert, AgentTeamTask,
    AgentTeamTaskUpsert, AgentTeamUpsert, ClaimOutcome, LedgerResult, RunEvent, RunEventAppend,
    RunEventListRequest, RunEventListResponse, RunTelemetry, RunTelemetryUpsert, WorkflowRun,
    WorkflowRunUpsert,
};

use super::with_ledger;

/// Inserts or merges an agent run.
pub fn upsert_agent_run(workspace_dir: &Path, upsert: AgentRunUpsert) -> LedgerResult<AgentRun> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_agent_run(upsert).await
    })
}

/// One agent run by id.
pub fn get_agent_run(workspace_dir: &Path, id: &str) -> LedgerResult<Option<AgentRun>> {
    let id = id.to_string();
    with_ledger(workspace_dir, |l| async move { l.get_agent_run(&id).await })
}

/// Inserts or merges a workflow run.
pub fn upsert_workflow_run(
    workspace_dir: &Path,
    upsert: WorkflowRunUpsert,
) -> LedgerResult<WorkflowRun> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_workflow_run(upsert).await
    })
}

/// One workflow run by id.
pub fn get_workflow_run(workspace_dir: &Path, id: &str) -> LedgerResult<Option<WorkflowRun>> {
    let id = id.to_string();
    with_ledger(
        workspace_dir,
        |l| async move { l.get_workflow_run(&id).await },
    )
}

/// Appends an event to a run's log.
pub fn append_run_event(workspace_dir: &Path, event: RunEventAppend) -> LedgerResult<RunEvent> {
    with_ledger(
        workspace_dir,
        |l| async move { l.append_run_event(event).await },
    )
}

/// A run's events in sequence order.
pub fn list_recent_run_events(
    workspace_dir: &Path,
    request: &RunEventListRequest,
) -> LedgerResult<RunEventListResponse> {
    let request = request.clone();
    with_ledger(workspace_dir, |l| async move {
        l.list_recent_run_events(&request).await
    })
}

/// Merges a telemetry rollup into a run's.
pub fn upsert_run_telemetry(
    workspace_dir: &Path,
    upsert: RunTelemetryUpsert,
) -> LedgerResult<RunTelemetry> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_run_telemetry(upsert).await
    })
}

/// Inserts or updates a team.
pub fn upsert_agent_team(workspace_dir: &Path, upsert: AgentTeamUpsert) -> LedgerResult<AgentTeam> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_agent_team(upsert).await
    })
}

/// Inserts or updates a team member.
pub fn upsert_agent_team_member(
    workspace_dir: &Path,
    upsert: AgentTeamMemberUpsert,
) -> LedgerResult<AgentTeamMember> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_agent_team_member(upsert).await
    })
}

/// Inserts or updates a team task.
pub fn upsert_agent_team_task(
    workspace_dir: &Path,
    upsert: AgentTeamTaskUpsert,
) -> LedgerResult<AgentTeamTask> {
    with_ledger(workspace_dir, |l| async move {
        l.upsert_agent_team_task(upsert).await
    })
}

/// One team task by id.
pub fn get_agent_team_task(workspace_dir: &Path, id: &str) -> LedgerResult<Option<AgentTeamTask>> {
    let id = id.to_string();
    with_ledger(workspace_dir, |l| async move {
        l.get_agent_team_task(&id).await
    })
}

/// One team member by id.
pub fn get_agent_team_member(
    workspace_dir: &Path,
    id: &str,
) -> LedgerResult<Option<AgentTeamMember>> {
    let id = id.to_string();
    with_ledger(workspace_dir, |l| async move {
        l.get_agent_team_member(&id).await
    })
}

/// Atomically claims a task for a member.
pub fn claim_agent_team_task(
    workspace_dir: &Path,
    team_id: &str,
    task_id: &str,
    member_id: &str,
    claim_token: &str,
) -> LedgerResult<ClaimOutcome> {
    let (team_id, task_id, member_id, claim_token) = (
        team_id.to_string(),
        task_id.to_string(),
        member_id.to_string(),
        claim_token.to_string(),
    );
    with_ledger(workspace_dir, |l| async move {
        l.claim_agent_team_task(&team_id, &task_id, &member_id, &claim_token)
            .await
    })
}

/// Marks a member as running a task.
pub fn mark_agent_team_member_running(
    workspace_dir: &Path,
    team_id: &str,
    member_id: &str,
    task_id: &str,
    worker_thread_id: &str,
    run_id: &str,
) -> LedgerResult<Option<AgentTeamMember>> {
    let (team_id, member_id, task_id, worker_thread_id, run_id) = (
        team_id.to_string(),
        member_id.to_string(),
        task_id.to_string(),
        worker_thread_id.to_string(),
        run_id.to_string(),
    );
    with_ledger(workspace_dir, |l| async move {
        l.mark_agent_team_member_running(&team_id, &member_id, &task_id, &worker_thread_id, &run_id)
            .await
    })
}
