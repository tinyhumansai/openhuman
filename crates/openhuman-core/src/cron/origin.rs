//! The conversation a cron job was created from: capturing it from the live
//! turn, deciding whether creating a job from it needs approval, and the turn
//! origin a run of such a job executes under.
//!
//! A job created inside a chat remembers that chat (a [`JobOrigin`]) so its
//! output can come back to it. The same snapshot carries trust: a job made
//! from an external channel keeps running as that channel's turn, so its own
//! external-effect tools stay gated exactly like its creator's were.

use crate::agent::turn_origin::{self, AgentTurnOrigin};
use serde_json::Value;
use {
    tinyflows_schedule::types::delivery_mode, tinyflows_schedule::types::CronJob,
    tinyflows_schedule::types::JobOrigin,
};

/// Map a live turn origin onto the origin a job created in it records.
///
/// Only a real conversation maps: a web thread, or a channel turn from the
/// channel processor (the one place that sets `history_key`). Every other
/// origin (CLI, automation, MCP server callers, triage, voice) has no chat to
/// return to and yields `None`.
pub(crate) fn job_origin_from_turn(origin: &AgentTurnOrigin) -> Option<JobOrigin> {
    match origin {
        AgentTurnOrigin::WebChat { thread_id, .. } if !thread_id.trim().is_empty() => {
            Some(JobOrigin::Web {
                thread_id: thread_id.clone(),
                agent_id: None,
            })
        }
        AgentTurnOrigin::ExternalChannel {
            channel,
            sender,
            reply_target,
            history_key: Some(history_key),
            ..
        } if !channel.trim().is_empty() && !reply_target.trim().is_empty() => {
            Some(JobOrigin::Channel {
                channel: channel.clone(),
                reply_target: reply_target.clone(),
                history_key: history_key.clone(),
                sender: sender.clone(),
                thread_id: None,
            })
        }
        _ => None,
    }
}

/// The origin a job created by the current turn would record, read from the
/// task-local [`AgentTurnOrigin`]. `None` outside a turn or in a turn with no
/// conversation behind it.
pub fn current_job_origin() -> Option<JobOrigin> {
    let origin = turn_origin::current()?;
    let job_origin = job_origin_from_turn(&origin);
    tracing::debug!(
        turn_origin = %origin.class(),
        captured = job_origin.as_ref().map(JobOrigin::kind_str).unwrap_or("none"),
        "[cron] origin capture"
    );
    job_origin
}

/// The turn origin a run of `job` executes under.
///
/// A job with a channel origin runs as that channel's turn
/// (`ExternalChannel`, message id `cron:<job>:<run>`), so the approval gate
/// treats its external-effect tools like the creator's. Everything else keeps
/// the user-authorized `TrustedAutomation { Cron }` it always had.
pub(crate) fn turn_origin_for_job_run(job: &CronJob, run_id: &str) -> AgentTurnOrigin {
    match &job.origin {
        Some(JobOrigin::Channel {
            channel,
            reply_target,
            history_key,
            sender,
            ..
        }) => AgentTurnOrigin::ExternalChannel {
            sender_name: None,
            channel: channel.clone(),
            sender: sender.clone(),
            reply_target: reply_target.clone(),
            message_id: format!("cron:{}:{run_id}", job.id),
            history_key: Some(history_key.clone()),
        },
        _ => AgentTurnOrigin::TrustedAutomation {
            job_id: job.id.clone(),
            source: turn_origin::TrustedAutomationSource::Cron,
        },
    }
}

fn non_empty_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Whether a scheduling call may skip the approval gate.
///
/// True only when ALL hold:
/// - the turn is a real external-channel conversation (`ExternalChannel` with a
///   history key),
/// - the job is an agent job with no shell `command`,
/// - its delivery resolves to the same conversation: no `delivery` (the
///   default is `origin`), `mode: "origin"`, or `announce` whose `channel` and
///   `to` equal the origin's channel and reply target.
///
/// The asker is then the only recipient, and the job's runs stay gated as that
/// channel's turns ([`turn_origin_for_job_run`]), so nothing is escalated.
/// Anything else (shell jobs, other recipients, `proactive`/`none` delivery,
/// non-channel origins) returns `false` and is gated as before
/// (GHSA-f46p-6vf9-64mm).
pub(crate) fn is_self_scoped_agent_job(args: &Value, turn: Option<&AgentTurnOrigin>) -> bool {
    let Some(JobOrigin::Channel {
        channel,
        reply_target,
        ..
    }) = turn.and_then(job_origin_from_turn)
    else {
        return false;
    };

    if non_empty_str(args, "command").is_some() {
        return false;
    }
    let is_agent = match args.get("job_type").and_then(Value::as_str) {
        Some("agent") => true,
        Some(_) => false,
        None => non_empty_str(args, "prompt").is_some(),
    };
    if !is_agent {
        return false;
    }

    // The exemption is sound only if the created job keeps its channel origin
    // (`create_agent_job` drops it unless the session target is `current` or
    // the delivery mode is `origin`): an origin-less job would run with
    // `TrustedAutomation`, not as this channel's `ExternalChannel` turn.
    let session_is_current = match args.get("session_target") {
        None | Some(Value::Null) => true,
        Some(value) => value
            .as_str()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("current")),
    };
    let delivery = match args.get("delivery") {
        None | Some(Value::Null) => return true,
        Some(delivery) => delivery,
    };
    let mode = non_empty_str(delivery, "mode")
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match mode.as_str() {
        delivery_mode::ORIGIN => true,
        delivery_mode::ANNOUNCE => {
            session_is_current
                && non_empty_str(delivery, "channel")
                    .is_some_and(|c| c.eq_ignore_ascii_case(&channel))
                && non_empty_str(delivery, "to") == Some(reply_target.as_str())
        }
        _ => false,
    }
}

/// [`is_self_scoped_agent_job`] against the live turn, with a log line for the
/// decision so an exempted creation is traceable.
pub(crate) fn current_turn_may_skip_approval(args: &Value) -> bool {
    let turn = turn_origin::current();
    let exempt = is_self_scoped_agent_job(args, turn.as_ref());
    if exempt {
        tracing::debug!("[cron] self-scoped agent job from channel turn; skipping approval gate");
    }
    exempt
}

#[cfg(test)]
#[path = "origin_tests.rs"]
mod tests;
