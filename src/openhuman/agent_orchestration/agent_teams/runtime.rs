//! Live execution runtime for agent-team members (#3374 PR4).
//!
//! PR1–PR3 shipped the durable team model, race-safe claiming, quality-gated
//! completion, and the read-only board. They were entirely *store-only*: a
//! claim flipped a row, nothing ran. This module makes a teammate actually
//! **execute** — [`start_member_run`] atomically claims a task for a member,
//! marks the member `active`, and spawns a background worker that drives a real
//! sub-agent to completion, captures its output as the task's completion
//! evidence, runs it through the existing quality gate, and returns the member
//! to `idle`.
//!
//! ## Root parent context
//!
//! Like the workflow engine (#3375), the worker is spawned from a controller
//! background task with no enclosing agent turn, so it builds a root
//! [`ParentExecutionContext`] (shared [`build_root_parent`]) and runs inside
//! [`with_parent_context`] — every nested `spawn_agent` then resolves a real
//! provider / tools / memory.
//!
//! ## Message delivery boundary
//!
//! Pending lead/teammate messages addressed to the member are injected into the
//! worker's prompt **at spawn** (a well-defined boundary), and are always
//! visible in the team timeline. Mid-turn injection into an already-running
//! harness loop is intentionally **not** supported — the orchestration layer has
//! no live inbox (`AgentOrchestrationSession::message_agent` records metadata
//! only). Boundary delivery satisfies the issue's "see the message in the team
//! timeline or worker thread" criterion via both paths; a live inbox would be a
//! separate orchestration-layer change.

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::openhuman::agent::harness::fork_context::with_parent_context;
use crate::openhuman::agent_orchestration::parent_context::build_root_parent;
use crate::openhuman::agent_orchestration::{
    AgentOrchestrationSession, AgentStatus, SpawnAgentRequest, WaitAgentOptions,
};
use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
    self, AgentTeamMemberStatus, AgentTeamTask, AgentTeamTaskStatus, ClaimOutcome, RunEvent,
    RunEventAppend, RunEventListRequest,
};

use super::types::{StartMemberOutcome, TeamError};

const LOG_TARGET: &str = "agent_team_runtime";
/// Fallback worker archetype when a member carries no explicit `agent_id`.
const DEFAULT_TEAMMATE_AGENT_ID: &str = "researcher";
const TEAM_MESSAGE_EVENT: &str = "team_message";
/// Per-member delivery watermark event: payload `{ memberId, upToSeq }`. Stored
/// on the same run-event log as the messages, so no schema change is needed.
const MESSAGE_DELIVERED_EVENT: &str = "team_message_delivered";
/// Event recorded when a worker run ends without completing its task.
const MEMBER_FAILED_EVENT: &str = "team_member_failed";
/// Page size when draining the full run-event log for a team. `list_recent_run_events`
/// returns `sequence ASC` from the cursor and caps `limit` at 1000, so a team whose
/// event count exceeds one page MUST be paged or later events (watermarks + messages)
/// are silently dropped.
const EVENT_PAGE_SIZE: u32 = 1000;
/// Cap on how much worker output is captured as evidence (UTF-8 safe).
const EVIDENCE_MAX_CHARS: usize = 280;

/// Start a live run for a team member. **Non-blocking.**
///
/// Resolves a target task (the explicit `task_id`, else the member's next
/// claimable ready task), atomically claims it, marks the member `active` with
/// the new `run_id`, and `tokio::spawn`s the worker loop — then returns
/// immediately. The UI observes progress by polling `agent_team_get`.
///
/// Returns a non-`Started` [`StartMemberOutcome`] (no side effects) when no work
/// could be dispatched: the task is already claimed, blocked on dependencies,
/// unknown, or the member has nothing claimable. An unknown member surfaces as
/// [`TeamError::UnknownMember`].
pub async fn start_member_run(
    config: &Config,
    team_id: &str,
    member_id: &str,
    task_id: Option<&str>,
    model_override: Option<String>,
) -> Result<StartMemberOutcome> {
    log::debug!(
        target: LOG_TARGET,
        "[agent_team_runtime] start.entry team={team_id} member={member_id} task={task_id:?}"
    );

    let member = run_ledger::get_agent_team_member(config, member_id)?
        .filter(|m| m.team_id == team_id)
        .ok_or_else(|| {
            anyhow!(TeamError::UnknownMember {
                member_id: member_id.to_string(),
            })
        })?;

    // Reject a start on an already-active member before any claim or state
    // mutation. The UI hides the control for active members, but this entry
    // point is reachable directly over RPC; without this guard two near-
    // simultaneous calls would each claim a task and the second
    // `mark_agent_team_member_running` would clobber the first's task/run
    // pointer, leaving two workers for one member.
    if member.member_status == AgentTeamMemberStatus::Active {
        log::debug!(
            target: LOG_TARGET,
            "[agent_team_runtime] start.reject_active team={team_id} member={member_id}"
        );
        return Ok(StartMemberOutcome::AlreadyActive);
    }

    // Resolve the target task: an explicit id, or the member's next claimable
    // ready task (unowned or owned-by-this-member, dependencies all done).
    let tasks = run_ledger::list_agent_team_tasks(config, team_id)?;
    let target = match task_id {
        Some(tid) => match tasks.iter().find(|t| t.id == tid) {
            Some(t) => t.clone(),
            None => return Ok(StartMemberOutcome::UnknownTask),
        },
        None => match pick_claimable(&tasks, member_id) {
            Some(t) => t.clone(),
            None => return Ok(StartMemberOutcome::NoClaimableTask),
        },
    };

    // The team-run id doubles as the claim token (CAS guard) and the member's
    // worker/run pointer surfaced to the UI.
    let run_id = format!("teamrun-{}", uuid::Uuid::new_v4().simple());
    let claimed =
        match run_ledger::claim_agent_team_task(config, team_id, &target.id, member_id, &run_id)? {
            ClaimOutcome::Claimed(task) => *task,
            ClaimOutcome::AlreadyClaimed => return Ok(StartMemberOutcome::AlreadyClaimed),
            ClaimOutcome::Blocked { unmet } => return Ok(StartMemberOutcome::Blocked { unmet }),
            ClaimOutcome::UnknownTask => return Ok(StartMemberOutcome::UnknownTask),
        };

    // Mark active synchronously so the polling UI reflects the running member
    // before the (async) worker even starts.
    run_ledger::mark_agent_team_member_running(
        config,
        team_id,
        member_id,
        &claimed.id,
        &run_id,
        &run_id,
    )?;

    let agent_id = member
        .agent_id
        .clone()
        .unwrap_or_else(|| DEFAULT_TEAMMATE_AGENT_ID.to_string());
    let cfg = config.clone();
    let team = team_id.to_string();
    let mem = member_id.to_string();
    let task_for_loop = claimed.clone();
    let rid = run_id.clone();
    tokio::spawn(async move {
        run_member_loop(
            &cfg,
            &team,
            &mem,
            &agent_id,
            task_for_loop,
            &rid,
            model_override,
        )
        .await;
    });

    log::debug!(
        target: LOG_TARGET,
        "[agent_team_runtime] start.spawned team={team_id} member={member_id} task={} run={run_id}",
        claimed.id
    );
    Ok(StartMemberOutcome::Started {
        run_id,
        task: Box::new(claimed),
    })
}

/// Build the root parent context, then drive the member's worker inside it.
/// Engine-internal failures (config/agent build, spawn, wait) release the task
/// and idle the member so the work is reclaimable; they are recorded, not
/// propagated (there is no caller on the spawned task).
async fn run_member_loop(
    config: &Config,
    team_id: &str,
    member_id: &str,
    agent_id: &str,
    task: AgentTeamTask,
    run_id: &str,
    model_override: Option<String>,
) {
    let outcome = match build_root_parent(config, "agent_team_runtime", "team", "teamrun").await {
        Ok(parent) => {
            with_parent_context(parent, async {
                drive_member(
                    config,
                    team_id,
                    member_id,
                    agent_id,
                    &task,
                    run_id,
                    model_override,
                )
                .await
            })
            .await
        }
        Err(err) => Err(err),
    };

    if let Err(err) = outcome {
        log::error!(
            target: LOG_TARGET,
            "[agent_team_runtime] loop.failed team={team_id} member={member_id} task={} err={err}",
            task.id
        );
        let _ = run_ledger::release_agent_team_task(config, team_id, &task.id);
        let _ = run_ledger::mark_agent_team_member_idle(config, team_id, member_id);
        record_failure_event(config, team_id, member_id, &task.id, &err.to_string());
    }
}

/// Spawn the worker sub-agent for `task`, wait for it, and reconcile team state.
///
/// Returns `Err` only for engine-internal failures (spawn/wait), which the
/// caller turns into a release + idle. A worker that runs but ends non-completed
/// is handled here (release + idle + failure event) and returns `Ok`.
async fn drive_member(
    config: &Config,
    team_id: &str,
    member_id: &str,
    agent_id: &str,
    task: &AgentTeamTask,
    run_id: &str,
    model_override: Option<String>,
) -> Result<()> {
    let delivered = deliver_pending_messages(config, team_id, member_id)?;
    let prompt = build_member_prompt(task, &delivered);

    let session = AgentOrchestrationSession::new(format!("team-{team_id}-{member_id}"));
    let resp = session
        .spawn_agent(SpawnAgentRequest {
            agent_id: agent_id.to_string(),
            prompt,
            model: model_override,
            metadata: [
                ("teamId".to_string(), team_id.to_string()),
                ("memberId".to_string(), member_id.to_string()),
                ("taskId".to_string(), task.id.clone()),
                ("teamRunId".to_string(), run_id.to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        })
        .await
        .map_err(|e| anyhow!("spawn teammate worker failed: {e}"))?;

    let wait = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec![resp.orchestration_id.clone()],
            timeout_ms: None,
        })
        .await
        .map_err(|e| anyhow!("wait teammate worker failed: {e}"))?;

    let snapshot = wait
        .agents
        .into_iter()
        .find(|a| a.orchestration_id == resp.orchestration_id)
        .ok_or_else(|| anyhow!("worker snapshot missing after wait"))?;

    match snapshot.status {
        AgentStatus::Completed => {
            let output = snapshot.result_summary.unwrap_or_default();
            let evidence = if output.trim().is_empty() {
                Vec::new()
            } else {
                vec![format!(
                    "run:{run_id} — {}",
                    truncate_chars(output.trim(), EVIDENCE_MAX_CHARS)
                )]
            };
            // The teammate's own output is the completion evidence, so the gate
            // does not additionally require evidence; it still enforces the
            // dependency / claimant invariants.
            let outcome = run_ledger::complete_agent_team_task(
                config, team_id, &task.id, member_id, &evidence, false,
            )?;
            log::debug!(
                target: LOG_TARGET,
                "[agent_team_runtime] drive.completed team={team_id} member={member_id} task={} outcome={outcome:?}",
                task.id
            );
            run_ledger::mark_agent_team_member_idle(config, team_id, member_id)?;
            Ok(())
        }
        AgentStatus::Failed | AgentStatus::Cancelled | AgentStatus::Closed => {
            let reason = snapshot
                .error
                .unwrap_or_else(|| "worker ended without completing".to_string());
            log::warn!(
                target: LOG_TARGET,
                "[agent_team_runtime] drive.worker_failed team={team_id} member={member_id} task={} reason={reason}",
                task.id
            );
            run_ledger::release_agent_team_task(config, team_id, &task.id)?;
            run_ledger::mark_agent_team_member_idle(config, team_id, member_id)?;
            record_failure_event(config, team_id, member_id, &task.id, &reason);
            Ok(())
        }
        other => {
            // `wait_agents` with no timeout only returns on terminal status, so
            // this is purely defensive.
            log::warn!(
                target: LOG_TARGET,
                "[agent_team_runtime] drive.non_terminal team={team_id} member={member_id} task={} status={other:?}",
                task.id
            );
            run_ledger::release_agent_team_task(config, team_id, &task.id)?;
            run_ledger::mark_agent_team_member_idle(config, team_id, member_id)?;
            Ok(())
        }
    }
}

/// Pick the member's next claimable task: the first (by order) task that is
/// `todo`/`ready`, unclaimed, owned by no-one or by this member, and whose
/// dependencies are all `done`.
fn pick_claimable<'a>(tasks: &'a [AgentTeamTask], member_id: &str) -> Option<&'a AgentTeamTask> {
    let done: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|t| t.status == AgentTeamTaskStatus::Done)
        .map(|t| t.id.as_str())
        .collect();
    tasks.iter().find(|t| {
        matches!(
            t.status,
            AgentTeamTaskStatus::Todo | AgentTeamTaskStatus::Ready
        ) && t.claimed_by_member_id.is_none()
            && t.owner_member_id
                .as_deref()
                .map(|o| o == member_id)
                .unwrap_or(true)
            && t.depends_on.iter().all(|d| done.contains(d.as_str()))
    })
}

/// Compose the worker prompt from the task + any pending messages addressed to
/// the member.
fn build_member_prompt(task: &AgentTeamTask, messages: &[String]) -> String {
    let mut prompt = format!("You are a teammate on an agent team. Task: {}", task.title);
    if let Some(obj) = task.objective.as_deref() {
        if !obj.trim().is_empty() {
            prompt.push_str("\n\nObjective:\n");
            prompt.push_str(obj.trim());
        }
    }
    if !messages.is_empty() {
        prompt.push_str("\n\nMessages from your lead / teammates:\n");
        for msg in messages {
            prompt.push_str("- ");
            prompt.push_str(msg);
            prompt.push('\n');
        }
    }
    prompt.push_str("\n\nComplete the task and report what you did.");
    prompt
}

/// Drain the entire `sequence ASC` run-event log for a team by paging past the
/// per-query cap. A single `list_recent_run_events` call returns at most 1000
/// rows from the cursor, so message delivery MUST page or a team that exceeds
/// one page would lose every watermark and message beyond it.
fn drain_run_events(config: &Config, team_id: &str) -> Result<Vec<RunEvent>> {
    let mut events: Vec<RunEvent> = Vec::new();
    let mut after: Option<u64> = None;
    loop {
        let response = run_ledger::list_recent_run_events(
            config,
            &RunEventListRequest {
                run_id: team_id.to_string(),
                after_sequence: after,
                limit: Some(EVENT_PAGE_SIZE),
            },
        )?;
        let exhausted = (response.count as u32) < EVENT_PAGE_SIZE;
        after = response.events.last().map(|e| e.sequence);
        events.extend(response.events);
        if exhausted || after.is_none() {
            break;
        }
    }
    Ok(events)
}

/// Read undelivered messages addressed to `member_id` (direct or broadcast),
/// advance the per-member delivery watermark, and return their contents. Uses
/// only the existing run-event log — no schema change.
fn deliver_pending_messages(
    config: &Config,
    team_id: &str,
    member_id: &str,
) -> Result<Vec<String>> {
    let events = drain_run_events(config, team_id)?;

    let watermark = events
        .iter()
        .filter(|e| e.event_type == MESSAGE_DELIVERED_EVENT)
        .filter(|e| e.payload.get("memberId").and_then(|v| v.as_str()) == Some(member_id))
        .filter_map(|e| e.payload.get("upToSeq").and_then(|v| v.as_i64()))
        .max()
        .unwrap_or(0);

    let mut max_seq = watermark;
    let mut contents = Vec::new();
    for event in &events {
        if event.event_type != TEAM_MESSAGE_EVENT || (event.sequence as i64) <= watermark {
            continue;
        }
        let to = event.payload.get("to").and_then(|v| v.as_str());
        // Direct (to == member) or broadcast (to absent/null).
        if to.is_none() || to == Some(member_id) {
            if let Some(content) = event.payload.get("content").and_then(|v| v.as_str()) {
                contents.push(content.to_string());
            }
            max_seq = max_seq.max(event.sequence as i64);
        }
    }

    if !contents.is_empty() {
        run_ledger::append_run_event(
            config,
            RunEventAppend {
                run_id: team_id.to_string(),
                event_type: MESSAGE_DELIVERED_EVENT.to_string(),
                payload: json!({ "memberId": member_id, "upToSeq": max_seq }),
            },
        )?;
    }
    Ok(contents)
}

fn record_failure_event(
    config: &Config,
    team_id: &str,
    member_id: &str,
    task_id: &str,
    reason: &str,
) {
    let _ = run_ledger::append_run_event(
        config,
        RunEventAppend {
            run_id: team_id.to_string(),
            event_type: MEMBER_FAILED_EVENT.to_string(),
            payload: json!({
                "memberId": member_id,
                "taskId": task_id,
                "reason": truncate_chars(reason, EVIDENCE_MAX_CHARS),
            }),
        },
    );
}

/// UTF-8-safe truncation by character count (never splits a codepoint).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
