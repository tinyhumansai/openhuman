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
//! no live inbox. Boundary delivery satisfies the issue's "see the message in the team
//! timeline or worker thread" criterion via both paths; a live inbox would be a
//! separate orchestration-layer change.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::agent::orchestration::parent_context::with_root_parent;
use crate::config::Config;
use tinyagents_session::run_ledger::{
    self, AgentTeamMemberStatus, AgentTeamTask, ClaimOutcome, RunEventAppend,
};
use {
    crate::agent::orchestration::AgentOrchestrationSession,
    crate::agent::orchestration::SpawnAgentRequest, crate::agent::orchestration::WaitAgentOptions,
    tinyagents_tasks::OrchestrationTaskStatus,
};

use tinyagents_orchestration::teams::{
    build_member_prompt, claimable_task, deliver_pending_messages, run_member_graph,
    truncate_chars, MemberOutcome, SessionTeamLedger, TeamError,
};

use crate::agent::tinyagents::observability::GraphTracingSink;

const LOG_TARGET: &str = "agent_team_runtime";
/// Fallback archetype when a member carries no explicit `agent_id`: the
/// orchestrator, the one general-purpose agent with the full direct belt plus
/// `tool_search` and skills. Same fallback a cron job with an unknown agent
/// uses (`cron/scheduler/agent_run.rs`).
const DEFAULT_TEAMMATE_AGENT_ID: &str = "orchestrator";
/// Event recorded when a worker run ends without completing its task.
const MEMBER_FAILED_EVENT: &str = "team_member_failed";
/// Cap on how much worker output is captured as evidence (UTF-8 safe).
const EVIDENCE_MAX_CHARS: usize = 280;

/// Host-side result of accepting a request to run a durable team member.
///
/// Actual worker execution remains OpenHuman-specific because it creates a
/// root parent context, selects a model and tools, and emits host progress.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum StartMemberOutcome {
    Started {
        run_id: String,
        task: Box<AgentTeamTask>,
    },
    Blocked {
        unmet: Vec<String>,
    },
    AlreadyClaimed,
    AlreadyActive,
    NoClaimableTask,
    UnknownTask,
}

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

    let member = run_ledger::get_agent_team_member(&config.workspace_dir, member_id)?
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
    let tasks = run_ledger::list_agent_team_tasks(&config.workspace_dir, team_id)?;
    let target = match task_id {
        Some(tid) => match tasks.iter().find(|t| t.id == tid) {
            Some(t) => t.clone(),
            None => return Ok(StartMemberOutcome::UnknownTask),
        },
        None => match claimable_task(&tasks, member_id) {
            Some(t) => t.clone(),
            None => return Ok(StartMemberOutcome::NoClaimableTask),
        },
    };

    // The team-run id doubles as the claim token (CAS guard) and the member's
    // worker/run pointer surfaced to the UI.
    let run_id = format!("teamrun-{}", uuid::Uuid::new_v4().simple());
    let claimed = match run_ledger::claim_agent_team_task(
        &config.workspace_dir,
        team_id,
        &target.id,
        member_id,
        &run_id,
    )? {
        ClaimOutcome::Claimed(task) => *task,
        ClaimOutcome::AlreadyClaimed => return Ok(StartMemberOutcome::AlreadyClaimed),
        ClaimOutcome::Blocked { unmet } => return Ok(StartMemberOutcome::Blocked { unmet }),
        ClaimOutcome::UnknownTask => return Ok(StartMemberOutcome::UnknownTask),
    };

    // Mark active synchronously so the polling UI reflects the running member
    // before the (async) worker even starts.
    run_ledger::mark_agent_team_member_running(
        &config.workspace_dir,
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
    // The member loop drives real agent work on a fresh task, and task-locals
    // don't cross `tokio::spawn` — capture the caller's turn origin here so the
    // worker keeps the label the approval gate needs. Inherit-only: no origin
    // in scope means the worker stays unlabelled and fails closed as before.
    let inherited_origin = crate::agent::turn_origin::capture();
    crate::core::runtime::spawn_scoped(async move {
        crate::agent::turn_origin::with_inherited_origin(
            inherited_origin,
            run_member_loop(
                &cfg,
                &team,
                &mem,
                &agent_id,
                task_for_loop,
                &rid,
                model_override,
            ),
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
    // The member acts on memory as itself within the team: its node is
    // `team:<team>/agent:<agent>`, so members share the team's node and
    // never read each other's.
    let identity = crate::memory::scope::MemoryIdentity::team_member(team_id, agent_id);
    let member_turn = with_root_parent(config, "agent_team_runtime", "team", "teamrun", async {
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
    });
    let outcome = crate::memory::scope::within(identity, Box::pin(member_turn))
        .await
        // Flatten: outer Err = root-parent build failure, inner = drive_member result.
        .unwrap_or_else(Err);

    if let Err(err) = outcome {
        log::error!(
            target: LOG_TARGET,
            "[agent_team_runtime] loop.failed team={team_id} member={member_id} task={} err={err}",
            task.id
        );
        let _ = run_ledger::release_agent_team_task(&config.workspace_dir, team_id, &task.id);
        let _ = run_ledger::mark_agent_team_member_idle(&config.workspace_dir, team_id, member_id);
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
    let ledger = SessionTeamLedger::new(config.workspace_dir.clone());
    let delivered = deliver_pending_messages(&ledger, team_id, member_id)?;
    let prompt = build_member_prompt(task, &delivered.messages);

    let session = AgentOrchestrationSession::new(format!("team-{team_id}-{member_id}"));

    // ── Worker node effect: spawn the teammate sub-agent, wait for it, and
    // classify the terminal outcome. Returns `Err` only for engine-internal
    // spawn/wait failures (the caller releases + idles).
    let run_worker = {
        let session = session.clone();
        let agent_id = agent_id.to_string();
        let team_id = team_id.to_string();
        let member_id = member_id.to_string();
        let task_id = task.id.clone();
        let run_id = run_id.to_string();
        move || {
            let session = session.clone();
            let agent_id = agent_id.clone();
            let prompt = prompt.clone();
            let model = model_override.clone();
            let team_id = team_id.clone();
            let member_id = member_id.clone();
            let task_id = task_id.clone();
            let run_id = run_id.clone();
            async move {
                let resp = session
                    .spawn_agent(SpawnAgentRequest {
                        agent_id,
                        prompt,
                        model,
                        metadata: [
                            ("teamId".to_string(), team_id),
                            ("memberId".to_string(), member_id),
                            ("taskId".to_string(), task_id),
                            ("teamRunId".to_string(), run_id),
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

                Ok(match snapshot.status {
                    OrchestrationTaskStatus::Completed => MemberOutcome::Completed {
                        output: snapshot.result_summary.unwrap_or_default(),
                    },
                    OrchestrationTaskStatus::Failed
                    | OrchestrationTaskStatus::Cancelled
                    | OrchestrationTaskStatus::CancelRequested
                    | OrchestrationTaskStatus::TimedOut
                    | OrchestrationTaskStatus::Abandoned => MemberOutcome::Failed {
                        reason: snapshot
                            .error
                            .unwrap_or_else(|| "worker ended without completing".to_string()),
                    },
                    // `wait_agents` with no timeout only returns on terminal
                    // status, so this is purely defensive — treat as a failure.
                    other => MemberOutcome::Failed {
                        reason: format!("worker returned non-terminal status {other:?}"),
                    },
                })
            }
        }
    };

    // ── Complete node effect: the teammate's own output is the completion
    // evidence (the gate enforces dependency / claimant invariants but not
    // additional evidence), then idle the member.
    let on_complete = {
        let config = config.clone();
        let team_id = team_id.to_string();
        let member_id = member_id.to_string();
        let task_id = task.id.clone();
        let run_id = run_id.to_string();
        move |output: String| {
            let config = config.clone();
            let team_id = team_id.clone();
            let member_id = member_id.clone();
            let task_id = task_id.clone();
            let run_id = run_id.clone();
            async move {
                let evidence = if output.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![format!(
                        "run:{run_id} — {}",
                        truncate_chars(output.trim(), EVIDENCE_MAX_CHARS)
                    )]
                };
                let outcome = run_ledger::complete_agent_team_task(
                    &config.workspace_dir,
                    &team_id,
                    &task_id,
                    &member_id,
                    &evidence,
                    false,
                )?;
                log::debug!(
                    target: LOG_TARGET,
                    "[agent_team_runtime] drive.completed team={team_id} member={member_id} task={task_id} outcome={outcome:?}"
                );
                run_ledger::mark_agent_team_member_idle(
                    &config.workspace_dir,
                    &team_id,
                    &member_id,
                )?;
                Ok(())
            }
        }
    };

    // ── Fail node effect: release the task (so it is reclaimable), idle the
    // member, and record a failure event. Returns `Ok` (a ran-but-failed worker
    // is a normal terminal outcome, not an engine error).
    let on_failed = {
        let config = config.clone();
        let team_id = team_id.to_string();
        let member_id = member_id.to_string();
        let task_id = task.id.clone();
        move |reason: String| {
            let config = config.clone();
            let team_id = team_id.clone();
            let member_id = member_id.clone();
            let task_id = task_id.clone();
            async move {
                log::warn!(
                    target: LOG_TARGET,
                    "[agent_team_runtime] drive.worker_failed team={team_id} member={member_id} task={task_id} reason={reason}"
                );
                run_ledger::release_agent_team_task(&config.workspace_dir, &team_id, &task_id)?;
                run_ledger::mark_agent_team_member_idle(
                    &config.workspace_dir,
                    &team_id,
                    &member_id,
                )?;
                record_failure_event(&config, &team_id, &member_id, &task_id, &reason);
                Ok(())
            }
        }
    };

    run_member_graph(
        Some(Arc::new(GraphTracingSink::new(format!(
            "team:{team_id}:{member_id}"
        )))),
        run_worker,
        on_complete,
        on_failed,
    )
    .await
}

fn record_failure_event(
    config: &Config,
    team_id: &str,
    member_id: &str,
    task_id: &str,
    reason: &str,
) {
    let _ = run_ledger::append_run_event(
        &config.workspace_dir,
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

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
