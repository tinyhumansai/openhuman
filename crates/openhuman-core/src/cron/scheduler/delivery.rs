//! Output delivery for completed cron runs: chat delivery (proactive /
//! announce), the alerts-tab notification, and the morning-briefing copy.

use super::agent_run::EMPTY_AGENT_OUTPUT;
use super::failure_classification::AGENT_JOB_USER_FAILURE_MESSAGE;
use super::origin_delivery::{deliver_to_origin, is_suppressed_output};
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use anyhow::Result;
use chrono::Utc;
use {
    tinyflows_bus::schedule::types::delivery_mode, tinyflows_bus::schedule::types::CronJob,
    tinyflows_bus::schedule::types::DeliveryConfig, tinyflows_bus::schedule::types::DeliveryStatus,
    tinyflows_bus::schedule::types::JobType,
};

pub(super) const MORNING_BRIEFING_AGENT_ID: &str = "morning_briefing";
pub(super) const MORNING_BRIEFING_FAILURE_NOTIFICATION: &str = "Morning briefing could not run. Check your AI provider, API key, and connected apps, then run it again from Settings > Cron Jobs.";

pub(super) fn is_morning_briefing_job(job: &CronJob) -> bool {
    job.name.as_deref() == Some(MORNING_BRIEFING_AGENT_ID)
        || job.agent_id.as_deref() == Some(MORNING_BRIEFING_AGENT_ID)
}

pub(super) fn strip_openhuman_link_markup(input: &str) -> String {
    const OPEN_TAG: &str = "<openhuman-link";
    const CLOSE_TAG: &str = "</openhuman-link>";

    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(OPEN_TAG) {
        output.push_str(&rest[..start]);
        let tag_and_after = &rest[start..];

        let Some(open_end) = tag_and_after.find('>') else {
            output.push_str(tag_and_after);
            return output;
        };
        let label_and_after = &tag_and_after[open_end + 1..];

        let Some(close_start) = label_and_after.find(CLOSE_TAG) else {
            output.push_str(tag_and_after);
            return output;
        };

        output.push_str(&label_and_after[..close_start]);
        rest = &label_and_after[close_start + CLOSE_TAG.len()..];
    }

    output.push_str(rest);
    output
}

pub(super) fn cron_alert_body(job: &CronJob, output: &str) -> String {
    let trimmed = output.trim();
    if matches!(job.job_type, JobType::Agent)
        && trimmed == AGENT_JOB_USER_FAILURE_MESSAGE
        && is_morning_briefing_job(job)
    {
        return MORNING_BRIEFING_FAILURE_NOTIFICATION.to_string();
    }

    let body = strip_openhuman_link_markup(output);
    crate::util::truncate_with_ellipsis(&body, 512)
}

/// Public entry point for delivering a job's output via the configured
/// delivery mode (proactive / announce / origin). Called by `cron_run` ("Run
/// Now") so manual runs also push notifications and alerts. Manual runs are
/// treated as `success = true` so the user always sees the result they
/// explicitly triggered (empty output is still skipped). Returns how the
/// delivery went, for the run record.
pub async fn deliver_job(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
    output: &str,
) -> DeliveryStatus {
    match deliver_run(ctx, config, job, run_id, output, true).await {
        Ok(status) => status,
        Err(e) => {
            if job.delivery.best_effort {
                tracing::warn!("[cron] delivery failed (best_effort, Run Now): {e}");
            } else {
                tracing::warn!("[cron] delivery failed (Run Now): {e}");
            }
            DeliveryStatus::Failed
        }
    }
}

/// True when an agent job produced no meaningful text — blank output or the
/// [`EMPTY_AGENT_OUTPUT`] placeholder. Such runs are never injected into chat.
pub(super) fn cron_output_is_empty(output: &str) -> bool {
    output.trim().is_empty() || output == EMPTY_AGENT_OUTPUT
}

/// Whether a cron job's output should be injected into the user's chat thread.
/// Skips failed runs and empty/placeholder output; failures still surface in
/// the alerts tab and run history (handled separately by the caller).
pub(super) fn should_deliver_cron_output_to_chat(success: bool, output: &str) -> bool {
    success && !cron_output_is_empty(output)
}

/// Whether a completed cron run should surface in the alerts tab
/// (`/notifications`). Failures stay visible even when they produce no output;
/// only successful-but-empty runs are dropped entirely.
pub(super) fn cron_result_should_alert(success: bool, output: &str) -> bool {
    !success || !cron_output_is_empty(output)
}

/// [`deliver_run`] under a fresh run id, discarding the status.
#[cfg(test)]
pub(super) async fn deliver_if_configured(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    output: &str,
    success: bool,
) -> Result<()> {
    let run_id = uuid::Uuid::new_v4().to_string();
    deliver_run(ctx, config, job, &run_id, output, success)
        .await
        .map(|_| ())
}

pub(super) async fn deliver_run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
    output: &str,
    success: bool,
) -> Result<DeliveryStatus> {
    let delivery: &DeliveryConfig = &job.delivery;

    // Don't post failed or empty cron runs into the user's chat: a failed turn
    // (e.g. a transient network error) would otherwise deliver a canned
    // "Something went wrong" message into the conversation with no user
    // message behind it. Failures still reach the alerts tab (`push_cron_alert`)
    // and the run-history / health signals, which are recorded elsewhere.
    let is_empty = cron_output_is_empty(output);
    let deliver_to_chat = should_deliver_cron_output_to_chat(success, output);
    if !deliver_to_chat {
        tracing::debug!(
            job_id = %job.id,
            success,
            is_empty,
            "[cron] skipping chat delivery for failed/empty cron run"
        );
    }

    // A failed run must stay visible in /notifications regardless of delivery
    // mode — a no-delivery agent job that halts on a permanent config/billing
    // state (e.g. a keyless provider, TAURI-RUST-HCK) would otherwise fail
    // silently. A *successful* non-empty run only alerts in the delivering
    // modes (proactive/announce); a `none`-mode success stays silent (its
    // output lives in last_output only — the cron contract), so we don't spam
    // explicitly-silent background jobs with an unread alert every interval
    // (Codex #4166).
    let mode = delivery.mode.trim().to_ascii_lowercase();
    let delivers = matches!(
        mode.as_str(),
        delivery_mode::PROACTIVE | delivery_mode::ANNOUNCE | delivery_mode::ORIGIN
    );
    // An origin-mode run that deliberately said nothing (`NO_REPLY` or blank)
    // is not an event worth an alert.
    let intentionally_silent =
        success && mode == delivery_mode::ORIGIN && is_suppressed_output(output);
    let alert_to_notifications = cron_result_should_alert(success, output)
        && (!success || delivers)
        && !intentionally_silent;
    let alert_body = if is_empty {
        "Scheduled job failed without output."
    } else {
        output
    };

    let mut status = if delivers {
        DeliveryStatus::Suppressed
    } else {
        DeliveryStatus::NotRequested
    };

    match mode.as_str() {
        // Origin delivery — commit the reply into the conversation that
        // created the job and send it there once.
        delivery_mode::ORIGIN => {
            if deliver_to_chat {
                status = deliver_to_origin(ctx, config, job, run_id, output).await?;
            }
        }

        // Proactive delivery — the channels module decides where to send.
        // Used by morning briefings, welcome messages, and other
        // user-facing proactive agents.
        "proactive" => {
            if deliver_to_chat {
                let source = format!("cron:{}", job.id);
                tracing::debug!(
                    job_id = %job.id,
                    source = %source,
                    "[cron] publishing ProactiveMessageRequested event"
                );
                BUS.publish(DomainEvent::ProactiveMessageRequested {
                    source,
                    message: output.to_string(),
                    job_name: job.name.clone(),
                });
                status = DeliveryStatus::Delivered;
            }
        }

        // Announce delivery — the cron job specifies the exact channel
        // and target. Used for explicit channel-targeted output.
        "announce" if deliver_to_chat => {
            let channel = delivery
                .channel
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.channel is required for announce mode"))?;
            let target = delivery
                .to
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.to is required for announce mode"))?;

            tracing::debug!(
                job_id = %job.id,
                channel = %channel,
                target = %target,
                "[cron] publishing CronDeliveryRequested event"
            );
            BUS.publish(DomainEvent::CronDeliveryRequested {
                job_id: job.id.clone(),
                channel: channel.to_string(),
                target: target.to_string(),
                output: output.to_string(),
            });
            status = DeliveryStatus::Delivered;
        }

        // No delivery configured — output is stored in last_output only.
        // The failure still reaches the alerts tab via the hoisted
        // `push_cron_alert` below.
        _ => {}
    }

    // Surface in the alerts tab (/notifications) for any result that isn't a
    // successful-but-empty run — INDEPENDENT of delivery mode. A failed cron
    // job must stay visible to the user even when it has no chat delivery
    // configured (the common case: a keyless agent job failing "API key not
    // set", TAURI-RUST-HCK). Previously this fired only inside the proactive /
    // announce arms, so no-delivery jobs failed silently in /notifications.
    if alert_to_notifications {
        push_cron_alert(config, job, alert_body);
    }

    Ok(status)
}

/// Insert a notification into the alerts tab for a completed cron job.
pub(super) fn push_cron_alert(config: &Config, job: &CronJob, output: &str) {
    use crate::desktop::notifications::store as notif_store;
    use crate::desktop::notifications::types::{IntegrationNotification, NotificationStatus};

    let name = job.name.as_deref().unwrap_or("Cron job");
    let body = cron_alert_body(job, output);

    let notification = IntegrationNotification {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "cron".to_string(),
        account_id: Some(job.id.clone()),
        title: name.to_string(),
        body,
        raw_payload: serde_json::json!({
            "job_id": job.id,
            "job_name": job.name,
            "delivery_mode": job.delivery.mode,
        }),
        importance_score: Some(0.65),
        triage_action: Some("react".to_string()),
        triage_reason: Some("Scheduled delivery".to_string()),
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: Some(Utc::now()),
    };

    match notif_store::insert_if_not_recent(config, &notification) {
        Ok(true) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] pushed notification alert to alerts tab"
            );
        }
        Ok(false) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] skipped duplicate notification alert"
            );
        }
        Err(e) => {
            tracing::warn!(
                job_id = %job.id,
                error = %e,
                "[cron] failed to push notification alert"
            );
        }
    }
}
