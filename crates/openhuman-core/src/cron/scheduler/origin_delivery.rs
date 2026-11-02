//! `delivery.mode = "origin"`: commit a run's reply into the conversation that
//! asked for it and send it there once.
//!
//! - Empty output is recorded as [`DeliveryStatus::Suppressed`]; nothing is
//!   sent and no alert is raised.
//! - A [`NO_REPLY`] answer (any spelling [`is_no_reply`] accepts) is suppressed
//!   the same way only when the job opted into silence by naming the token in
//!   its own prompt ([`job_allows_no_reply`]). Otherwise the token is never
//!   sent: the host's [`fallback_reminder`] is delivered in its place.
//! - Web origin: the reply is stored in the origin thread first, under the
//!   deterministic id of `run_reply_message_id("cron:<job>:<run>")`, then
//!   announced on the web channel as a `proactive_message` event addressed to
//!   that thread. Re-delivering the same run lands on the same row.
//! - Channel origin: the reply is sent to the origin's reply target, then
//!   appended to its durable agent transcript so the next turn sees it.

#[cfg(feature = "channels")]
use super::delivery::strip_openhuman_link_markup;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
#[cfg(feature = "channels")]
use crate::cron::channel_bridge;
use crate::web_chat::{publish_web_channel_event, WebChannelEvent};
use anyhow::{anyhow, Result};
use std::path::Path;
use tinyflows_bus::schedule::types::{CronJob, DeliveryStatus, JobOrigin, JobType};

/// What a scheduled run replies when it has nothing worth sending.
pub(crate) const NO_REPLY: &str = "NO_REPLY";

/// True when `output` is the [`NO_REPLY`] token in any common spelling: any
/// case, `_`, `-` or a space between the words, wrapped in quotes, backticks
/// or emphasis, with trailing punctuation.
pub(crate) fn is_no_reply(output: &str) -> bool {
    let core = output.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | '*' | '_' | '.' | '!' | '?' | ',' | ';' | ':'
            )
    });
    let folded: String = core
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    folded == "noreply"
}

/// True when the job may end a run with [`NO_REPLY`]: its prompt names the
/// token (a conditional check such as "otherwise reply NO_REPLY"). Shell and
/// flow output is not model-written, so those jobs always may.
pub(crate) fn job_allows_no_reply(job: &CronJob) -> bool {
    if !matches!(job.job_type, JobType::Agent) {
        return true;
    }
    job.prompt
        .as_deref()
        .is_some_and(|prompt| prompt.to_ascii_lowercase().contains("no_reply"))
}

/// The reminder the host sends when a run that must deliver produced no
/// message: the job's name, else its prompt.
pub(crate) fn fallback_reminder(job: &CronJob) -> String {
    let subject = job
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            job.prompt
                .as_deref()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(|prompt| crate::util::truncate_with_ellipsis(prompt, 200))
        });
    match subject {
        Some(subject) => format!("Reminder: {subject}"),
        None => "Reminder".to_string(),
    }
}

/// The text a run delivers, or `None` when it intentionally sends nothing
/// (blank output, or [`NO_REPLY`] from a job that allows it). A disallowed
/// `NO_REPLY` becomes the job's [`fallback_reminder`].
pub(crate) fn deliverable_reply(job: &CronJob, output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_no_reply(trimmed) {
        return (!job_allows_no_reply(job)).then(|| fallback_reminder(job));
    }
    Some(trimmed.to_string())
}

/// The kind of conversation a job delivers to, for logs.
pub(crate) fn origin_kind(job: &CronJob) -> &'static str {
    match job.origin {
        Some(JobOrigin::Web { .. }) => "web",
        Some(JobOrigin::Channel { .. }) => "channel",
        None => "none",
    }
}

/// The deterministic request id of one run's delivered reply.
pub(crate) fn origin_request_id(job_id: &str, run_id: &str) -> String {
    format!("cron:{job_id}:{run_id}")
}

/// Everything the web agent-transcript append needs about one delivered reply.
pub(crate) struct TranscriptAppend<'a> {
    pub session_key: &'a str,
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
/// Uses `tinyagents_session::transcript::append_background_message_in`: it
/// waits for any live turn of the session (the per-session turn lock), dedupes
/// on the idempotency key and records `{kind: "cron", job_id, run_id}`
/// provenance. The session is addressed exactly as the session host does it
/// (`SessionRef::scoped(session_key, agent_id)`) in `transcripts`, the store
/// the host wrote it to, which the caller resolved from the job's tenant
/// ([`origin_transcripts`]). A thread that never had an agent turn has no
/// transcript (`NoSession`) and is skipped. Must never be called from inside a
/// turn of the same session.
pub(crate) async fn append_to_origin_transcript(
    transcripts: &dyn tinyagents_session::transcript::TranscriptLocator,
    append: &TranscriptAppend<'_>,
) -> Result<(), String> {
    use tinyagents_session::transcript::{
        append_background_message_in, BackgroundAppend, BackgroundAppendOutcome, SessionRef,
        TranscriptMessage,
    };
    let session = SessionRef::scoped(append.session_key.to_string(), append.agent_id.to_string());
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
        append_background_message_in(
            transcripts,
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

/// The transcript store a web thread's turns were written to: the session
/// store under [`agent_key_for`](crate::storage::sessions::agent_key_for)
/// of the job's tenant and the thread's agent, the key the session host
/// resolved for the same tenant.
pub(crate) fn origin_transcripts(
    workspace_dir: &Path,
    tenant: &crate::core::runtime::Tenant,
    agent_id: &str,
) -> std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
    let key = crate::storage::sessions::agent_key_for(tenant, agent_id);
    crate::storage::sessions::transcripts_or_files(&key, workspace_dir)
}

/// Deliver one run's output to the job's origin conversation.
///
/// `Ok(Suppressed)` when the run intentionally sends nothing (see
/// [`deliverable_reply`]); `Ok(Delivered)` when the reply was committed and
/// sent; `Err` when it could not be.
pub(crate) async fn deliver_to_origin(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
    output: &str,
) -> Result<DeliveryStatus> {
    let Some(reply) = deliverable_reply(job, output) else {
        tracing::info!(
            job_id = %job.id,
            run_id = %run_id,
            origin = origin_kind(job),
            "[cron] origin delivery suppressed: the run had nothing to send"
        );
        return Ok(DeliveryStatus::Suppressed);
    };
    let Some(origin) = job.origin.as_ref() else {
        return Err(anyhow!(
            "delivery mode 'origin' requires the job to have an origin conversation"
        ));
    };
    let text = reply.as_str();
    if let JobOrigin::Channel {
        channel,
        channel_session,
        ..
    } = origin
    {
        let identity = channel_session
            .as_deref()
            .ok_or_else(|| anyhow!("channel origin has no durable session identity"))?;
        super::origin_context::validated_channel_session(ctx, config, channel, identity)
            .map_err(|error| anyhow!("invalid channel origin: {error}"))?;
    }

    // The job's tenant: a hosted profile's or an embedded agent's host routes
    // the reply itself; otherwise it keys the transcript store below.
    let tenant = ctx.tenant().ok();
    let owner = tenant
        .clone()
        .filter(|tenant| tenant.profile.is_some() || tenant.agent.is_some());
    if let (Some(sink), Some(owner)) = (crate::cron::host_delivery::installed(), owner) {
        let thread_id = match origin {
            JobOrigin::Web { thread_id, .. } => Some(thread_id.clone()),
            JobOrigin::Channel { thread_id, .. } => thread_id.clone(),
        };
        tracing::debug!(
            job_id = %job.id,
            run_id = %run_id,
            profile = owner.profile.is_some(),
            agent = owner.agent.is_some(),
            "[cron] origin delivery handed to the host"
        );
        sink.deliver(crate::cron::host_delivery::CronReply {
            profile_id: owner.profile,
            agent_id: owner.agent,
            job_id: job.id.clone(),
            run_id: run_id.to_string(),
            thread_id,
            origin: Some(origin.clone()),
            text: text.to_string(),
        })
        .await
        .map_err(|e| anyhow!("the host could not deliver the reply: {e}"))?;
        if let JobOrigin::Channel {
            channel,
            channel_session: Some(identity),
            ..
        } = origin
        {
            append_channel_transcript(ctx, config, channel, identity, job, run_id, text).await?;
        }
        return Ok(DeliveryStatus::Delivered);
    }

    match origin {
        JobOrigin::Web {
            thread_id,
            agent_id,
        } => {
            deliver_to_web_thread(
                ctx,
                config,
                tenant.as_ref(),
                job,
                run_id,
                thread_id,
                agent_id.as_deref(),
                text,
            )
            .await
        }
        JobOrigin::Channel {
            channel,
            reply_target,
            channel_session,
            ..
        } => {
            #[cfg(not(feature = "channels"))]
            {
                let _ = (channel, reply_target, channel_session);
                return Err(anyhow!("channel delivery requires the channels feature"));
            }
            #[cfg(feature = "channels")]
            {
                let identity = channel_session
                    .as_deref()
                    .ok_or_else(|| anyhow!("channel origin has no durable session identity"))?;
                let (session_key, agent_id, _tenant) =
                    super::origin_context::validated_channel_session(
                        ctx, config, channel, identity,
                    )
                    .map_err(|error| anyhow!("invalid channel origin: {error}"))?;
                let text = strip_openhuman_link_markup(text);
                channel_bridge::send_to_channel(
                    channel,
                    identity,
                    reply_target,
                    &text,
                    &origin_request_id(&job.id, run_id),
                )
                .await
                .map_err(|e| anyhow!(e))?;
                let transcripts = origin_channel_transcripts(ctx, config, identity, &agent_id)?;
                append_to_origin_transcript(
                    transcripts.as_ref(),
                    &TranscriptAppend {
                        session_key: &session_key,
                        agent_id: &agent_id,
                        text: &text,
                        idempotency_key: &origin_request_id(&job.id, run_id),
                        job_id: &job.id,
                        run_id,
                    },
                )
                .await
                .map_err(|error| anyhow!("could not persist channel origin reply: {error}"))?;
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
}

async fn append_channel_transcript(
    ctx: &DispatchCtx,
    config: &Config,
    channel: &str,
    identity: &tinyflows_bus::schedule::ChannelSessionIdentity,
    job: &CronJob,
    run_id: &str,
    text: &str,
) -> Result<()> {
    let (session_key, agent_id, _) =
        super::origin_context::validated_channel_session(ctx, config, channel, identity)
            .map_err(|error| anyhow!("invalid channel origin: {error}"))?;
    let transcripts = origin_channel_transcripts(ctx, config, identity, &agent_id)?;
    append_to_origin_transcript(
        transcripts.as_ref(),
        &TranscriptAppend {
            session_key: &session_key,
            agent_id: &agent_id,
            text,
            idempotency_key: &origin_request_id(&job.id, run_id),
            job_id: &job.id,
            run_id,
        },
    )
    .await
    .map_err(|error| anyhow!("could not persist channel origin reply: {error}"))
}

fn origin_channel_transcripts(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &tinyflows_bus::schedule::ChannelSessionIdentity,
    agent_id: &str,
) -> Result<std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>> {
    let core = ctx
        .core()
        .ok_or_else(|| anyhow!("channel origin has no resolved core context"))?;
    let tenant = ctx
        .tenant()
        .map_err(|error| anyhow!("could not resolve channel origin tenant: {error}"))?;
    if identity.tenant_key != crate::core::runtime::tenant_key(&tenant, "") {
        return Err(anyhow!("channel origin belongs to a different tenant"));
    }
    Ok(crate::storage::sessions::transcripts_or_files_in(
        core,
        &crate::storage::sessions::agent_key_for(&tenant, agent_id),
        &config.workspace_dir,
    ))
}

async fn deliver_to_web_thread(
    ctx: &DispatchCtx,
    config: &Config,
    tenant: Option<&crate::core::runtime::Tenant>,
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
    let persist_ctx = ctx.clone();
    persist_ctx
        .clone()
        .spawn_blocking(move || {
            crate::web_chat::persist_delivered_reply(
                &persist_ctx,
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

    publish_web_channel_event(WebChannelEvent::ProactiveMessage {
        context: crate::web_chat::event_context("system", thread_id, request_id.clone()),
        full_response: Some(text.to_string()),
        message: None,
        success: Some(true),
        persisted_message_id: Some(crate::threads::store::run_reply_message_id(&request_id)),
        parts: None,
    });

    // After the announcement: waiting out a live turn of this session must not
    // delay the user seeing the reply (the thread row is already stored).
    // A job with no tenant (SaaS work that lost its scope) has no store to
    // append to and must not guess one.
    let Some(tenant) = tenant else {
        tracing::warn!(
            job_id = %job.id,
            run_id = %run_id,
            "[cron] origin transcript append skipped: the job has no tenant"
        );
        return Ok(DeliveryStatus::Delivered);
    };
    let transcripts = origin_transcripts(&config.workspace_dir, tenant, &agent_id);
    if let Err(e) = append_to_origin_transcript(
        transcripts.as_ref(),
        &TranscriptAppend {
            session_key: thread_id,
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

#[cfg(all(test, feature = "channels"))]
#[path = "origin_delivery_channel_tests.rs"]
mod channel_tests;
