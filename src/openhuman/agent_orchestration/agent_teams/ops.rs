//! Business logic for durable agent-team coordination (#3374).
//!
//! Thin orchestration over `session_db::run_ledger`: create teams + members,
//! assign dependency-aware tasks (with self/unknown/cycle validation reusing
//! the same Kahn's-algorithm shape as `workflow_runs`), atomically claim tasks,
//! and exchange teammate messages. Messaging rides the run-ledger event stream
//! (`run_id = team_id`, `event_type = "team_message"`) — no new message table.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
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
        config,
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
            config,
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
    run_ledger::list_agent_teams(config, request)
}

/// Build the aggregate [`TeamView`] for a team id; `None` if the team is absent.
pub fn get_team(config: &Config, team_id: &str) -> Result<Option<TeamView>> {
    log::debug!("{LOG_PREFIX} get_team.entry id={team_id}");
    match run_ledger::get_agent_team(config, team_id)? {
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

    let team = run_ledger::get_agent_team(config, team_id)?
        .ok_or_else(|| anyhow!("unknown team: {team_id}"))?;
    let _ = team;

    let existing = run_ledger::list_agent_team_tasks(config, team_id)?;
    let task_id = format!("task-{}", Uuid::new_v4().simple());

    if let Some(owner) = owner_member_id {
        let members = run_ledger::list_agent_team_members(config, team_id)?;
        if !members.iter().any(|m| m.id == owner) {
            return Err(anyhow!(TeamError::UnknownMember {
                member_id: owner.to_string(),
            }));
        }
    }

    validate_dependencies(&task_id, depends_on, &existing)?;

    let order_index = existing.len() as i64;
    let task = run_ledger::upsert_agent_team_task(
        config,
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
    let members = run_ledger::list_agent_team_members(config, team_id)?;
    if !members.iter().any(|m| m.id == member_id) {
        return Err(anyhow!(TeamError::UnknownMember {
            member_id: member_id.to_string(),
        }));
    }
    run_ledger::claim_agent_team_task(config, team_id, task_id, member_id, claim_token)
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
    if run_ledger::get_agent_team(config, team_id)?.is_none() {
        return Err(anyhow!("unknown team: {team_id}"));
    }

    let members = run_ledger::list_agent_team_members(config, team_id)?;
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
        config,
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
        config,
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
    let existing = run_ledger::get_agent_team(config, team_id)?
        .ok_or_else(|| anyhow!("unknown team: {team_id}"))?;
    let team = run_ledger::upsert_agent_team(
        config,
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
    let members = run_ledger::list_agent_team_members(config, team_id)?;
    if !members.iter().any(|m| m.id == member_id) {
        return Err(anyhow!(TeamError::UnknownMember {
            member_id: member_id.to_string(),
        }));
    }
    let outcome = run_ledger::complete_agent_team_task(
        config,
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
        run_ledger::shutdown_agent_team_member(config, team_id, member_id)?.ok_or_else(|| {
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
    let team = run_ledger::get_agent_team(config, team_id)?
        .ok_or_else(|| anyhow!("team missing after creation: {team_id}"))?;
    let members = run_ledger::list_agent_team_members(config, team_id)?;
    let tasks = run_ledger::list_agent_team_tasks(config, team_id)?;
    Ok(TeamView {
        team,
        members,
        tasks,
    })
}

/// Validate a new task's dependency edges against the team's existing tasks.
///
/// Rejects self-dependency, unknown dependency ids, and any edge that would
/// introduce a cycle. The cycle check builds the full graph (existing tasks +
/// the new task with its proposed deps) and runs Kahn's algorithm — the same
/// shape used for workflow phase graphs in `workflow_runs::ops::has_cycle`.
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

/// Kahn's-algorithm cycle check over the task dependency graph (existing tasks
/// plus the candidate new task). Edge `dep -> task` means `task` depends on
/// `dep`. Edges pointing at unknown ids are ignored here (rejected separately).
fn has_task_cycle(
    new_task_id: &str,
    new_depends_on: &[String],
    existing: &[AgentTeamTask],
) -> bool {
    // Node set: every existing task id plus the new one.
    let mut nodes: HashSet<&str> = existing.iter().map(|t| t.id.as_str()).collect();
    nodes.insert(new_task_id);

    let mut indegree: HashMap<&str, usize> = nodes.iter().map(|&n| (n, 0)).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();

    // Existing edges.
    for task in existing {
        for dep in &task.depends_on {
            let dep = dep.as_str();
            if nodes.contains(dep) {
                adjacency.entry(dep).or_default().push(task.id.as_str());
                *indegree.entry(task.id.as_str()).or_insert(0) += 1;
            }
        }
    }
    // Candidate edges for the new task.
    for dep in new_depends_on {
        let dep = dep.as_str();
        if nodes.contains(dep) {
            adjacency.entry(dep).or_default().push(new_task_id);
            *indegree.entry(new_task_id).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(children) = adjacency.get(node) {
            for &child in children {
                let entry = indegree.get_mut(child).expect("child in indegree");
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(child);
                }
            }
        }
    }
    visited != indegree.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> Config {
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.action_dir = dir.path().join("actions");
        config
    }

    fn team_err(err: anyhow::Error) -> TeamError {
        err.downcast::<TeamError>().expect("TeamError")
    }

    #[test]
    fn create_team_rejects_duplicate_member_names() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let err = create_team(
            &config,
            "lead",
            None,
            None,
            &[
                NewMember {
                    name: "alice".into(),
                    agent_id: None,
                },
                NewMember {
                    name: "alice".into(),
                    agent_id: None,
                },
            ],
        )
        .unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::DuplicateMemberName {
                name: "alice".into()
            }
        );
    }

    #[test]
    fn assign_task_rejects_self_unknown_and_cycle() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let view = create_team(
            &config,
            "lead",
            None,
            None,
            &[NewMember {
                name: "alice".into(),
                agent_id: None,
            }],
        )
        .unwrap();
        let team_id = view.team.id.clone();

        // Unknown dependency.
        let err =
            assign_task(&config, &team_id, "task one", None, None, &["ghost".into()]).unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::UnknownDependency {
                depends_on: "ghost".into()
            }
        );

        // Seed A, then B depends_on A — fine.
        let a = assign_task(&config, &team_id, "A", None, None, &[]).unwrap();
        let b = assign_task(&config, &team_id, "B", None, None, &[a.id.clone()]).unwrap();

        // Self-dependency.
        let err = assign_task(&config, &team_id, "self", None, None, &["task-xyz".into()]);
        // self id is generated, so simulate via an existing-task edit path instead:
        // unknown id path already covered; ensure self check fires when dep == new id.
        // We can't predict the new id; verify cycle path instead.
        let _ = err;

        // Cycle: try to make A depend_on B (A already an upstream of B).
        // Re-upserting A with depends_on [B] would close the loop; assign_task
        // only creates new tasks, so emulate the cycle check directly.
        let existing = run_ledger::list_agent_team_tasks(&config, &team_id).unwrap();
        assert!(has_task_cycle(&a.id, &[b.id.clone()], &existing));
    }

    #[test]
    fn self_dependency_is_rejected() {
        // Directly exercise validate_dependencies with a matching id.
        let err = validate_dependencies("task-self", &["task-self".into()], &[]).unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::SelfDependency {
                task_id: "task-self".into()
            }
        );
    }

    #[test]
    fn message_append_then_list_in_order() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let view = create_team(
            &config,
            "lead",
            None,
            None,
            &[
                NewMember {
                    name: "alice".into(),
                    agent_id: None,
                },
                NewMember {
                    name: "bob".into(),
                    agent_id: None,
                },
            ],
        )
        .unwrap();
        let team_id = view.team.id.clone();
        let alice = view.members[0].id.clone();
        let bob = view.members[1].id.clone();

        message_member(&config, &team_id, Some(&alice), Some(&bob), "first", None).unwrap();
        message_member(&config, &team_id, Some(&bob), Some(&alice), "second", None).unwrap();

        let messages = list_messages(&config, &team_id, None).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].sequence, 1);
        assert_eq!(messages[1].sequence, 2);
        assert_eq!(messages[0].payload["content"], "first");
        assert_eq!(messages[1].payload["content"], "second");
    }

    #[test]
    fn message_member_lead_origin_and_unknown_from() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, alice) = solo_team(&config, "alice");

        // Lead/user-originated message: from = None → stored as "lead", no
        // member validation on the sender.
        message_member(&config, &team_id, None, Some(&alice), "from the lead", None).unwrap();
        let messages = list_messages(&config, &team_id, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload["from"], LEAD_SENDER);
        assert_eq!(messages[0].payload["to"], alice);

        // A non-None sender that is not a member is still rejected.
        let err =
            message_member(&config, &team_id, Some("ghost"), Some(&alice), "x", None).unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::UnknownMember {
                member_id: "ghost".into()
            }
        );
    }

    /// Create a single-member team and return `(team_id, member_id)`.
    fn solo_team(config: &Config, name: &str) -> (String, String) {
        let view = create_team(
            config,
            "lead",
            None,
            None,
            &[NewMember {
                name: name.into(),
                agent_id: None,
            }],
        )
        .unwrap();
        let member_id = view.members[0].id.clone();
        (view.team.id, member_id)
    }

    #[test]
    fn complete_task_gate_passes_and_marks_done() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, alice) = solo_team(&config, "alice");

        let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
        let claim = claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();
        assert!(matches!(claim, ClaimOutcome::Claimed(_)));

        let outcome = complete_task(
            &config,
            &team_id,
            &task.id,
            &alice,
            &["https://ci/run/1".to_string()],
            true,
        )
        .unwrap();
        match outcome {
            CompletionOutcome::Completed(done) => {
                assert_eq!(done.status, AgentTeamTaskStatus::Done);
                assert_eq!(done.gate_status, "passed");
                assert_eq!(done.gate_reason, None);
                assert_eq!(done.evidence, vec!["https://ci/run/1".to_string()]);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn complete_task_requires_evidence_then_recovers() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, alice) = solo_team(&config, "alice");

        let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
        claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

        // No evidence + require_evidence → gate fails, task stays in progress.
        let failed = complete_task(&config, &team_id, &task.id, &alice, &[], true).unwrap();
        match failed {
            CompletionOutcome::GateFailed { reasons } => {
                assert!(
                    reasons.iter().any(|r| r.contains("evidence")),
                    "{reasons:?}"
                );
            }
            other => panic!("expected GateFailed, got {other:?}"),
        }
        let mid = run_ledger::get_agent_team_task(&config, &task.id)
            .unwrap()
            .unwrap();
        assert_eq!(mid.status, AgentTeamTaskStatus::InProgress);
        assert_eq!(mid.gate_status, "failed");

        // Retry with evidence → passes.
        let ok = complete_task(
            &config,
            &team_id,
            &task.id,
            &alice,
            &["proof".to_string()],
            true,
        )
        .unwrap();
        assert!(matches!(ok, CompletionOutcome::Completed(_)));
    }

    #[test]
    fn complete_task_is_not_double_completable() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, alice) = solo_team(&config, "alice");

        let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
        claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

        let first = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
        assert!(matches!(first, CompletionOutcome::Completed(_)));

        // A task that is already `done` is no longer in progress, so a second
        // completion is rejected (the `status = 'in_progress'` UPDATE guard makes
        // the CAS airtight even under a concurrent double-complete).
        let second = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
        assert_eq!(second, CompletionOutcome::NotClaimed);
    }

    #[test]
    fn complete_task_rejects_non_claimant() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let view = create_team(
            &config,
            "lead",
            None,
            None,
            &[
                NewMember {
                    name: "alice".into(),
                    agent_id: None,
                },
                NewMember {
                    name: "bob".into(),
                    agent_id: None,
                },
            ],
        )
        .unwrap();
        let team_id = view.team.id.clone();
        let alice = view.members[0].id.clone();
        let bob = view.members[1].id.clone();

        let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
        claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

        // Bob is a member but not the claimant → NotClaimed.
        let outcome = complete_task(&config, &team_id, &task.id, &bob, &[], false).unwrap();
        assert_eq!(outcome, CompletionOutcome::NotClaimed);

        // Unknown member → typed error (not an outcome).
        let err = complete_task(&config, &team_id, &task.id, "ghost", &[], false).unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::UnknownMember {
                member_id: "ghost".into()
            }
        );
    }

    #[test]
    fn complete_task_owner_mismatch_fails_gate() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let view = create_team(
            &config,
            "lead",
            None,
            None,
            &[
                NewMember {
                    name: "alice".into(),
                    agent_id: None,
                },
                NewMember {
                    name: "bob".into(),
                    agent_id: None,
                },
            ],
        )
        .unwrap();
        let team_id = view.team.id.clone();
        let alice = view.members[0].id.clone();
        let bob = view.members[1].id.clone();

        // Task owned by bob, but alice claims + tries to complete.
        let task = assign_task(&config, &team_id, "ship it", None, Some(&bob), &[]).unwrap();
        claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

        let outcome = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
        match outcome {
            CompletionOutcome::GateFailed { reasons } => {
                assert!(
                    reasons.iter().any(|r| r.contains("owned by")),
                    "{reasons:?}"
                );
            }
            other => panic!("expected GateFailed, got {other:?}"),
        }
    }

    #[test]
    fn shutdown_member_releases_in_progress_tasks() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, alice) = solo_team(&config, "alice");

        let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
        claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

        let result = shutdown_member(&config, &team_id, &alice).unwrap();
        assert_eq!(result.released_task_ids, vec![task.id.clone()]);
        assert_eq!(result.member.member_status, AgentTeamMemberStatus::Stopped);

        // Task is back to todo and unclaimed → another teammate could claim it.
        let released = run_ledger::get_agent_team_task(&config, &task.id)
            .unwrap()
            .unwrap();
        assert_eq!(released.status, AgentTeamTaskStatus::Todo);
        assert_eq!(released.claimed_by_member_id, None);
        assert_eq!(released.claim_token, None);
    }

    #[test]
    fn shutdown_member_unknown_errors() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let (team_id, _alice) = solo_team(&config, "alice");

        let err = shutdown_member(&config, &team_id, "ghost").unwrap_err();
        assert_eq!(
            team_err(err),
            TeamError::UnknownMember {
                member_id: "ghost".into()
            }
        );
    }
}
