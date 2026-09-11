//! Business logic for durable agent-team coordination (#3374).
//!
//! Thin orchestration over `tinyagents_session::run_ledger`: create teams + members,
//! assign dependency-aware tasks (with self/unknown/cycle validation reusing
//! the same Kahn's-algorithm shape as `workflow_runs`), atomically claim tasks,
//! and exchange teammate messages. Messaging rides the run-ledger event stream
//! (`run_id = team_id`, `event_type = "team_message"`) — no new message table.

use std::collections::HashSet;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::openhuman::config::Config;
use tinyagents_graph::dag::{has_cycle, DagNode};
use tinyagents_session::run_ledger::{
    self, AgentTeam, AgentTeamListRequest, AgentTeamListResponse, AgentTeamMemberStatus,
    AgentTeamMemberUpsert, AgentTeamStatus, AgentTeamTask, AgentTeamTaskStatus,
    AgentTeamTaskUpsert, AgentTeamUpsert, ClaimOutcome, CompletionOutcome, RunEvent,
    RunEventAppend, RunEventListRequest,
};

use super::types::{MemberShutdown, TeamError, TeamView};

const LOG_PREFIX: &str = "[agent_team]";
const TEAM_MESSAGE_EVENT: &str = "team_message";

/// One member to create at team-creation time.
#[derive(Debug, Clone)]
pub struct NewMember {
    pub name: String,
    pub agent_id: Option<String>,
}

/// Create a team and its initial members.
///
/// Rejects duplicate member names ([`TeamError::DuplicateMemberName`]).
pub fn create_team(
    config: &Config,
    lead_agent_id: &str,
    parent_thread_id: Option<&str>,
    summary: Option<&str>,
    members: &[NewMember],
) -> Result<TeamView> {
    log::debug!(
        "{LOG_PREFIX} create_team.entry lead={lead_agent_id} members={}",
        members.len()
    );

    let mut seen: HashSet<&str> = HashSet::new();
    for member in members {
        if !seen.insert(member.name.as_str()) {
            return Err(anyhow!(TeamError::DuplicateMemberName {
                name: member.name.clone(),
            }));
        }
    }

    let team_id = format!("team-{}", Uuid::new_v4().simple());
    run_ledger::upsert_agent_team(
        &config.workspace_dir,
        AgentTeamUpsert {
            id: team_id.clone(),
            parent_thread_id: parent_thread_id.map(str::to_string),
            lead_agent_id: lead_agent_id.to_string(),
            status: AgentTeamStatus::Active,
            summary: summary.map(str::to_string),
            created_at: None,
            closed_at: None,
        },
    )?;

    for member in members {
        run_ledger::upsert_agent_team_member(
            &config.workspace_dir,
            AgentTeamMemberUpsert {
                id: format!("member-{}", Uuid::new_v4().simple()),
                team_id: team_id.clone(),
                name: member.name.clone(),
                agent_id: member.agent_id.clone(),
                member_status: AgentTeamMemberStatus::Pending,
                current_task_id: None,
                worker_thread_id: None,
                run_id: None,
                created_at: None,
            },
        )?;
    }

    let view = team_view(config, &team_id)?;
    log::debug!("{LOG_PREFIX} create_team.exit id={team_id}");
    Ok(view)
}

/// List teams (delegates to the run ledger).
pub fn list_teams(
    config: &Config,
    request: &AgentTeamListRequest,
) -> Result<AgentTeamListResponse> {
    log::debug!("{LOG_PREFIX} list_teams.entry status={:?}", request.status);
    Ok(run_ledger::list_agent_teams(
        &config.workspace_dir,
        request,
    )?)
}

/// Build the aggregate [`TeamView`] for a team id; `None` if the team is absent.
pub fn get_team(config: &Config, team_id: &str) -> Result<Option<TeamView>> {
    log::debug!("{LOG_PREFIX} get_team.entry id={team_id}");
    match run_ledger::get_agent_team(&config.workspace_dir, team_id)? {
        Some(_) => Ok(Some(team_view(config, team_id)?)),
        None => {
            log::debug!("{LOG_PREFIX} get_team.exit id={team_id} found=false");
            Ok(None)
        }
    }
}

/// Assign a new dependency-aware task to a team.
///
/// Validates `depends_on`: rejects self-dependency, unknown dependency ids, and
/// dependency cycles (Kahn's algorithm over the team's existing tasks plus the
/// new one). An optional `owner_member_id` must reference a real member.
#[allow(clippy::too_many_arguments)]
pub fn assign_task(
    config: &Config,
    team_id: &str,
    title: &str,
    objective: Option<&str>,
    owner_member_id: Option<&str>,
    depends_on: &[String],
) -> Result<AgentTeamTask> {
    log::debug!(
        "{LOG_PREFIX} assign_task.entry team={team_id} deps={}",
        depends_on.len()
    );

    let team = run_ledger::get_agent_team(&config.workspace_dir, team_id)?
        .ok_or_else(|| anyhow!("unknown team: {team_id}"))?;
    let _ = team;

    let existing = run_ledger::list_agent_team_tasks(&config.workspace_dir, team_id)?;
    let task_id = format!("task-{}", Uuid::new_v4().simple());

    if let Some(owner) = owner_member_id {
        let members = run_ledger::list_agent_team_members(&config.workspace_dir, team_id)?;
        if !members.iter().any(|m| m.id == owner) {
            return Err(anyhow!(TeamError::UnknownMember {
                member_id: owner.to_string(),
            }));
        }
    }

    validate_dependencies(&task_id, depends_on, &existing)?;

    let order_index = existing.len() as i64;
    let task = run_ledger::upsert_agent_team_task(
        &config.workspace_dir,
        AgentTeamTaskUpsert {
            id: task_id.clone(),
            team_id: team_id.to_string(),
            title: title.to_string(),
            objective: objective.map(str::to_string),
            status: AgentTeamTaskStatus::Todo,
            owner_member_id: owner_member_id.map(str::to_string),
            depends_on: depends_on.to_vec(),
            gate_status: None,
            gate_reason: None,
            evidence: vec![],
            source_run_id: None,
            order_index,
            created_at: None,
        },
    )?;
    log::debug!("{LOG_PREFIX} assign_task.exit team={team_id} task={task_id}");
    Ok(task)
}

/// Atomically claim a task for a member (delegates to the run-ledger CAS).
pub fn claim_task(
    config: &Config,
    team_id: &str,
    task_id: &str,
    member_id: &str,
    claim_token: &str,
) -> Result<ClaimOutcome> {
    log::debug!("{LOG_PREFIX} claim_task.entry team={team_id} task={task_id} member={member_id}");
    let members = run_ledger::list_agent_team_members(&config.workspace_dir, team_id)?;
    if !members.iter().any(|m| m.id == member_id) {
        return Err(anyhow!(TeamError::UnknownMember {
            member_id: member_id.to_string(),
        }));
    }
    Ok(run_ledger::claim_agent_team_task(
        &config.workspace_dir,
        team_id,
        task_id,
        member_id,
        claim_token,
    )?)
}

/// Sentinel `from` value for a message that originates from the team lead / the
/// human user rather than a teammate member. The UI send-composer uses this so a
/// person can address a named teammate without being a member row themselves.
pub const LEAD_SENDER: &str = "lead";

/// Send a message from one member to another (or broadcast).
///
/// `from_member_id = None` marks a lead/user-originated message (stored with
/// `from = "lead"`); `Some(id)` is a teammate-to-teammate message and must
/// reference a real member. Persisted as a run-ledger event keyed by
/// `run_id = team_id`, so the messaging stream reuses the durable event log with
/// no new table.
pub fn message_member(
    config: &Config,
    team_id: &str,
    from_member_id: Option<&str>,
    to_member_id: Option<&str>,
    content: &str,
    visibility: Option<&str>,
) -> Result<RunEvent> {
    log::debug!(
        "{LOG_PREFIX} message_member.entry team={team_id} from={:?} to={:?}",
        from_member_id,
        to_member_id
    );

    // Reject unknown teams up front. A lead-origin broadcast (`from = None`,
    // `to = None`) skips both member checks below, so without this guard an
    // unknown `team_id` would still append an orphan `team_message` event to a
    // non-existent team's run ledger.
    if run_ledger::get_agent_team(&config.workspace_dir, team_id)?.is_none() {
        return Err(anyhow!("unknown team: {team_id}"));
    }

    let members = run_ledger::list_agent_team_members(&config.workspace_dir, team_id)?;
    if let Some(from) = from_member_id {
        if !members.iter().any(|m| m.id == from) {
            return Err(anyhow!(TeamError::UnknownMember {
                member_id: from.to_string(),
            }));
        }
    }
    if let Some(to) = to_member_id {
        if !members.iter().any(|m| m.id == to) {
            return Err(anyhow!(TeamError::UnknownMember {
                member_id: to.to_string(),
            }));
        }
    }

    let from_value = from_member_id.unwrap_or(LEAD_SENDER);
    let event = run_ledger::append_run_event(
        &config.workspace_dir,
        RunEventAppend {
            run_id: team_id.to_string(),
            event_type: TEAM_MESSAGE_EVENT.to_string(),
            payload: json!({
                "from": from_value,
                "to": to_member_id,
                "content": content,
                "visibility": visibility.unwrap_or("team"),
            }),
        },
    )?;
    log::debug!(
        "{LOG_PREFIX} message_member.exit team={team_id} sequence={}",
        event.sequence
    );
    Ok(event)
}

/// List the team's message events in sequence order.
pub fn list_messages(config: &Config, team_id: &str, limit: Option<u32>) -> Result<Vec<RunEvent>> {
    log::debug!("{LOG_PREFIX} list_messages.entry team={team_id}");
    let response = run_ledger::list_recent_run_events(
        &config.workspace_dir,
        &RunEventListRequest {
            run_id: team_id.to_string(),
            after_sequence: None,
            limit,
        },
    )?;
    let messages: Vec<RunEvent> = response
        .events
        .into_iter()
        .filter(|e| e.event_type == TEAM_MESSAGE_EVENT)
        .collect();
    log::debug!(
        "{LOG_PREFIX} list_messages.exit team={team_id} count={}",
        messages.len()
    );
    Ok(messages)
}

/// Mark a team closed.
pub fn close_team(config: &Config, team_id: &str, summary: Option<&str>) -> Result<AgentTeam> {
    log::debug!("{LOG_PREFIX} close_team.entry team={team_id}");
    let existing = run_ledger::get_agent_team(&config.workspace_dir, team_id)?
        .ok_or_else(|| anyhow!("unknown team: {team_id}"))?;
    let team = run_ledger::upsert_agent_team(
        &config.workspace_dir,
        AgentTeamUpsert {
            id: team_id.to_string(),
            parent_thread_id: existing.parent_thread_id.clone(),
            lead_agent_id: existing.lead_agent_id.clone(),
            status: AgentTeamStatus::Closed,
            summary: summary.map(str::to_string),
            created_at: Some(existing.created_at),
            closed_at: Some(Utc::now()),
        },
    )?;
    log::debug!("{LOG_PREFIX} close_team.exit team={team_id}");
    Ok(team)
}

/// Complete a claimed task, gating its transition to `done`.
///
/// Validates the completing member belongs to the team, then delegates to the
/// run-ledger completion CAS, which enforces the quality gate (dependencies
/// done, claimant owns the task, evidence present when `require_evidence`) and
/// only flips the task to `done` when every invariant holds.
pub fn complete_task(
    config: &Config,
    team_id: &str,
    task_id: &str,
    member_id: &str,
    evidence: &[String],
    require_evidence: bool,
) -> Result<CompletionOutcome> {
    log::debug!(
        "{LOG_PREFIX} complete_task.entry team={team_id} task={task_id} member={member_id}"
    );
    let members = run_ledger::list_agent_team_members(&config.workspace_dir, team_id)?;
    if !members.iter().any(|m| m.id == member_id) {
        return Err(anyhow!(TeamError::UnknownMember {
            member_id: member_id.to_string(),
        }));
    }
    let outcome = run_ledger::complete_agent_team_task(
        &config.workspace_dir,
        team_id,
        task_id,
        member_id,
        evidence,
        require_evidence,
    )?;
    log::debug!("{LOG_PREFIX} complete_task.exit team={team_id} task={task_id}");
    Ok(outcome)
}

/// Stop a team member, releasing any task it was actively working on.
///
/// Unknown member ids surface as [`TeamError::UnknownMember`]; otherwise returns
/// the stopped member plus the ids of tasks released back to `todo`.
pub fn shutdown_member(config: &Config, team_id: &str, member_id: &str) -> Result<MemberShutdown> {
    log::debug!("{LOG_PREFIX} shutdown_member.entry team={team_id} member={member_id}");
    let (member, released_task_ids) =
        run_ledger::shutdown_agent_team_member(&config.workspace_dir, team_id, member_id)?
            .ok_or_else(|| {
                anyhow!(TeamError::UnknownMember {
                    member_id: member_id.to_string(),
                })
            })?;
    log::debug!(
        "{LOG_PREFIX} shutdown_member.exit team={team_id} member={member_id} released={}",
        released_task_ids.len()
    );
    Ok(MemberShutdown {
        member,
        released_task_ids,
    })
}

fn team_view(config: &Config, team_id: &str) -> Result<TeamView> {
    let team = run_ledger::get_agent_team(&config.workspace_dir, team_id)?
        .ok_or_else(|| anyhow!("team missing after creation: {team_id}"))?;
    let members = run_ledger::list_agent_team_members(&config.workspace_dir, team_id)?;
    let tasks = run_ledger::list_agent_team_tasks(&config.workspace_dir, team_id)?;
    Ok(TeamView {
        team,
        members,
        tasks,
    })
}

/// Validate a new task's dependency edges against the team's existing tasks.
///
/// Rejects self-dependency, unknown dependency ids, and any edge that would
/// introduce a cycle. The self and unknown checks stay here because they are
/// scoped to the *new* task: a pre-existing task carrying a dangling edge is
/// not this caller's fault and must not block the assignment. The cycle check
/// delegates to `tinyagents_graph::dag`.
fn validate_dependencies(
    new_task_id: &str,
    depends_on: &[String],
    existing: &[AgentTeamTask],
) -> Result<()> {
    let known: HashSet<&str> = existing.iter().map(|t| t.id.as_str()).collect();

    for dep in depends_on {
        if dep == new_task_id {
            return Err(anyhow!(TeamError::SelfDependency {
                task_id: new_task_id.to_string(),
            }));
        }
        if !known.contains(dep.as_str()) {
            return Err(anyhow!(TeamError::UnknownDependency {
                depends_on: dep.clone(),
            }));
        }
    }

    if has_task_cycle(new_task_id, depends_on, existing) {
        return Err(anyhow!(TeamError::CyclicDependency));
    }

    Ok(())
}

/// Cycle check over the task dependency graph (existing tasks plus the
/// candidate new task), delegating to `tinyagents_graph::dag::has_cycle`.
///
/// Edge `dep -> task` means `task` depends on `dep`. Edges pointing at unknown
/// ids are ignored by the shared validator (they are rejected separately).
fn has_task_cycle(
    new_task_id: &str,
    new_depends_on: &[String],
    existing: &[AgentTeamTask],
) -> bool {
    let mut nodes: Vec<DagNode<'_>> = existing
        .iter()
        .map(|t| DagNode::new(t.id.as_str(), t.depends_on.iter().map(String::as_str)))
        .collect();
    nodes.push(DagNode::new(
        new_task_id,
        new_depends_on.iter().map(String::as_str),
    ));
    has_cycle(&nodes)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
