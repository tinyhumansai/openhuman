//! `delivery.mode = "origin"`: commit a run's reply into the conversation that
//! asked for it and send it there once.
//!
//! - Empty output, or exactly [`NO_REPLY`], is an intentional "nothing to
//!   say": recorded as [`DeliveryStatus::Suppressed`], nothing is sent and no
//!   alert is raised.
//! - Web origin: the reply is stored in the origin thread first, under the
//!   deterministic id of `run_reply_message_id("cron:<job>:<run>")`, then
//!   announced on the web channel as a `proactive_message` event addressed to
//!   that thread. Re-delivering the same run lands on the same row.
//! - Channel origin: the reply is sent to the origin's reply target, then
//!   appended to that chat's history so its next turn sees it.

use super::delivery::strip_openhuman_link_markup;
use crate::config::Config;
use crate::web_chat::{publish_web_channel_event, WebChannelEvent};
use anyhow::{anyhow, Result};
use std::path::Path;
use {
    crate::cron::channel_bridge, tinyflows_schedule::types::CronJob,
    tinyflows_schedule::types::DeliveryStatus, tinyflows_schedule::types::JobOrigin,
};

/// What a scheduled run replies when it has nothing worth sending.
pub(crate) const NO_REPLY: &str = "NO_REPLY";

/// True when a run's output means "deliver nothing": blank, or exactly
/// [`NO_REPLY`] after trimming (case-sensitive).
pub(crate) fn is_suppressed_output(output: &str) -> bool {
    let trimmed = output.trim();
    trimmed.is_empty() || trimmed == NO_REPLY
}

/// The deterministic request id of one run's delivered reply.
pub(crate) fn origin_request_id(job_id: &str, run_id: &str) -> String {
    format!("cron:{job_id}:{run_id}")
}

/// Everything the web agent-transcript append needs about one delivered reply.
pub(crate) struct TranscriptAppend<'a> {
    pub thread_id: &'a str,
    /// The agent that owns the thread's session (its transcript is keyed by
    /// thread id and agent id).
    pub agent_id: &'a str,
    pub text: &'a str,
    /// `cron:<job_id>:<run_id>`; the append is idempotent on it.
    pub idempotency_key: &'a str,
    pub job_id: &'a str,
    pub run_id: &'a str,
}

/// How long a delivery waits for a live turn of the same session to finish
/// before giving up on the transcript append (the thread row is already stored).
const TRANSCRIPT_APPEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Append the delivered reply to the origin thread's **agent transcript** (the
/// model-facing `SessionRef` transcript) as an assistant message, so the next
/// live turn in that thread sees the reminder.
///
/// Uses `tinyagents_session::transcript::append_background_message`: it waits
/// for any live turn of the session (the per-session turn lock), dedupes on the
/// idempotency key and records `{kind: "cron", job_id, run_id}` provenance.
/// The session is addressed exactly as the session host does it
/// (`SessionRef::scoped(thread_id, agent_id)` over the workspace path). A
/// thread that never had an agent turn has no transcript (`NoSession`) and is
/// skipped. Must never be called from inside a turn of the same session.
pub(crate) async fn append_to_origin_transcript(
    workspace_dir: &Path,
    append: &TranscriptAppend<'_>,
) -> Result<(), String> {
    use tinyagents_session::transcript::{
        append_background_message, BackgroundAppend, BackgroundAppendOutcome,
        FileTranscriptLocator, SessionRef, TranscriptMessage,
    };
    let session = SessionRef::scoped(append.thread_id.to_string(), append.agent_id.to_string());
    if let Some(agent) = crate::agent::session_store::current_embedded_agent() {
        use tinyagents_session::transcript::TranscriptLocator;
        crate::agent::session_store::AgentTranscriptFiles::new(workspace_dir, &agent)
            .session_exists(&session);
    }
    let locator =
        FileTranscriptLocator::new(crate::agent::session_store::transcript_root(workspace_dir));
    let options = BackgroundAppend::new(
        append.idempotency_key,
        serde_json::json!({
            "kind": "cron",
            "job_id": append.job_id,
            "run_id": append.run_id,
        }),
    );
    let outcome = tokio::time::timeout(
        TRANSCRIPT_APPEND_TIMEOUT,
        append_background_message(
            &locator,
            &session,
            TranscriptMessage::assistant(append.text),
            options,
        ),
    )
    .await
    .map_err(|_| "timed out waiting for the thread's live turn".to_string())?
    .map_err(|e| e.to_string())?;
    match outcome {
        BackgroundAppendOutcome::Appended { generation } => tracing::debug!(
            job_id = %append.job_id,
            run_id = %append.run_id,
            generation,
            "[cron] appended reply to origin agent transcript"
        ),
        BackgroundAppendOutcome::Duplicate { generation } => tracing::debug!(
            job_id = %append.job_id,
            run_id = %append.run_id,
            generation,
            "[cron] origin agent transcript already holds this run's reply"
        ),
        BackgroundAppendOutcome::StaleGeneration { expected, head } => tracing::debug!(
            job_id = %append.job_id,
            expected,
            head,
            "[cron] origin agent transcript generation moved; append skipped"
        ),
        BackgroundAppendOutcome::NoSession => tracing::debug!(
            job_id = %append.job_id,
            "[cron] origin thread has no agent transcript yet; append skipped"
        ),
    }
    Ok(())
}

/// Deliver one run's output to the job's origin conversation.
///
/// `Ok(Suppressed)` for an intentional empty/`NO_REPLY` output; `Ok(Delivered)`
/// when the reply was committed and sent; `Err` when it could not be.
pub(crate) async fn deliver_to_origin(
    config: &Config,
    job: &CronJob,
    run_id: &str,
    output: &str,
) -> Result<DeliveryStatus> {
    if is_suppressed_output(output) {
        tracing::debug!(
            job_id = %job.id,
            run_id = %run_id,
            "[cron] origin delivery suppressed (empty or NO_REPLY)"
        );
        return Ok(DeliveryStatus::Suppressed);
    }
    let Some(origin) = job.origin.as_ref() else {
        return Err(anyhow!(
            "delivery mode 'origin' requires the job to have an origin conversation"
        ));
    };
    let text = output.trim();

    // A hosted profile's host routes the reply itself.
    if let (Some(sink), Some(profile_id)) = (
        crate::cron::host_delivery::installed(),
        crate::core::runtime::current_tenant()
            .ok()
            .and_then(|tenant| tenant.profile),
    ) {
        let thread_id = match origin {
            JobOrigin::Web { thread_id, .. } => Some(thread_id.clone()),
            JobOrigin::Channel { thread_id, .. } => thread_id.clone(),
        };
        sink.deliver(crate::cron::host_delivery::CronReply {
            profile_id,
            job_id: job.id.clone(),
            run_id: run_id.to_string(),
            thread_id,
            origin: Some(origin.clone()),
            text: text.to_string(),
        })
        .await
        .map_err(|e| anyhow!("the host could not deliver the reply: {e}"))?;
        return Ok(DeliveryStatus::Delivered);
    }

    match origin {
        JobOrigin::Web {
            thread_id,
            agent_id,
        } => deliver_to_web_thread(config, job, run_id, thread_id, agent_id.as_deref(), text).await,
        JobOrigin::Channel {
            channel,
            reply_target,
            history_key,
            thread_id,
            ..
        } => {
            let text = strip_openhuman_link_markup(text);
            channel_bridge::send_to_channel(
                channel,
                reply_target,
                thread_id.as_deref(),
                &text,
                &origin_request_id(&job.id, run_id),
            )
            .await
            .map_err(|e| anyhow!(e))?;
            channel_bridge::append_assistant_message(history_key, &text);
            tracing::debug!(
                job_id = %job.id,
                run_id = %run_id,
                channel = %channel,
                "[cron] origin delivery sent to channel"
            );
            Ok(DeliveryStatus::Delivered)
        }
    }
}

async fn deliver_to_web_thread(
    config: &Config,
    job: &CronJob,
    run_id: &str,
    thread_id: &str,
    agent_id: Option<&str>,
    text: &str,
) -> Result<DeliveryStatus> {
    let request_id = origin_request_id(&job.id, run_id);
    let agent_id = agent_id
        .map(str::to_string)
        .unwrap_or_else(|| crate::web_chat::pick_target_agent_id(config));

    // Store first: a reply that reached the announcement must already be on
    // disk, and a missing thread (deleted since the job was created) fails here
    // instead of announcing into nowhere.
    let workspace = config.workspace_dir.clone();
    let persist_thread = thread_id.to_string();
    let persist_request = request_id.clone();
    let persist_text = text.to_string();
    crate::core::runtime::spawn_blocking_scoped(move || {
        crate::web_chat::persist_delivered_reply(
            &workspace,
            &persist_thread,
            &persist_request,
            &persist_text,
            &[],
        )
    })
    .await
    .map_err(|e| anyhow!("origin reply persistence task failed: {e}"))?
    .map_err(|e| anyhow!("could not store the reply in the origin thread: {e}"))?;

    publish_web_channel_event(WebChannelEvent {
        event: "proactive_message".to_string(),
        client_id: "system".to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.clone(),
        full_response: Some(text.to_string()),
        success: Some(true),
        persisted_message_id: Some(crate::threads::store::run_reply_message_id(&request_id)),
        ..Default::default()
    });

    // After the announcement: waiting out a live turn of this session must not
    // delay the user seeing the reply (the thread row is already stored).
    if let Err(e) = append_to_origin_transcript(
        &config.workspace_dir,
        &TranscriptAppend {
            thread_id,
            agent_id: &agent_id,
            text,
            idempotency_key: &request_id,
            job_id: &job.id,
            run_id,
        },
    )
    .await
    {
        tracing::warn!(
            job_id = %job.id,
            run_id = %run_id,
            error = %e,
            "[cron] origin transcript append failed; thread row is stored"
        );
    }

    tracing::debug!(
        job_id = %job.id,
        run_id = %run_id,
        request_id = %request_id,
        "[cron] origin delivery stored and announced on web thread"
    );
    Ok(DeliveryStatus::Delivered)
}

#[cfg(test)]
#[path = "origin_delivery_tests.rs"]
mod tests;
