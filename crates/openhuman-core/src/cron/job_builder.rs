//! One path for agent-created agent jobs, shared by the `cron` and `schedule`
//! tools: origin-aware defaults, delivery validation, persistence.
//!
//! A job created inside a conversation returns its output to that
//! conversation: `session_target: current`, `delivery.mode: origin`, with the
//! conversation snapshot stored on the job. A job created outside one (CLI,
//! automation) keeps the old defaults, `isolated` + `proactive`. A target or
//! delivery the caller states explicitly always wins.

use crate::config::Config;
use {
    crate::cron, tinyflows_schedule::types::delivery_mode, tinyflows_schedule::types::CronJob,
    tinyflows_schedule::types::DeliveryConfig, tinyflows_schedule::types::JobOrigin,
    tinyflows_schedule::types::Schedule, tinyflows_schedule::types::SessionTarget,
    tinyflows_sqlite::schedule::AgentJobSpec,
};

/// What an agent job creation request carries; everything defaults.
pub struct AgentJobInput {
    pub name: Option<String>,
    pub schedule: Schedule,
    pub prompt: String,
    pub session_target: Option<SessionTarget>,
    pub model: Option<String>,
    pub delivery: Option<DeliveryConfig>,
    pub delete_after_run: bool,
}

/// The `allowed_users` list configured for a channel, by name. `None` when the
/// channel is unknown or unconfigured; an empty list means any sender.
fn allowed_users_for_channel<'a>(config: &'a Config, channel: &str) -> Option<&'a [String]> {
    let ch = channel.trim().to_ascii_lowercase();
    let cc = &config.channels_config;
    match ch.as_str() {
        "telegram" => cc.telegram.as_ref().map(|c| c.allowed_users.as_slice()),
        "discord" => cc.discord.as_ref().map(|c| c.allowed_users.as_slice()),
        "slack" => cc.slack.as_ref().map(|c| c.allowed_users.as_slice()),
        "mattermost" => cc.mattermost.as_ref().map(|c| c.allowed_users.as_slice()),
        "matrix" => cc.matrix.as_ref().map(|c| c.allowed_users.as_slice()),
        "irc" => cc.irc.as_ref().map(|c| c.allowed_users.as_slice()),
        "lark" => cc.lark.as_ref().map(|c| c.allowed_users.as_slice()),
        "dingtalk" => cc.dingtalk.as_ref().map(|c| c.allowed_users.as_slice()),
        "qq" => cc.qq.as_ref().map(|c| c.allowed_users.as_slice()),
        _ => None,
    }
}

/// The default delivery for an agent job: back to the conversation it was
/// created in, or the in-app stream when there is none.
pub fn default_delivery(origin: Option<&JobOrigin>) -> DeliveryConfig {
    DeliveryConfig {
        mode: if origin.is_some() {
            delivery_mode::ORIGIN
        } else {
            delivery_mode::PROACTIVE
        }
        .to_string(),
        channel: None,
        to: None,
        best_effort: true,
    }
}

/// Reject a session target or delivery that needs a conversation the job does
/// not have. Shared with the RPC `cron.add` path.
pub fn check_origin_requirements(
    session_target: &SessionTarget,
    delivery: &DeliveryConfig,
    origin: Option<&JobOrigin>,
) -> Result<(), String> {
    if origin.is_some() {
        return Ok(());
    }
    if delivery
        .mode
        .trim()
        .eq_ignore_ascii_case(delivery_mode::ORIGIN)
    {
        return Err(
            "delivery mode 'origin' needs the conversation the job is created from; this \
             job has none (use 'proactive' or 'announce')"
                .to_string(),
        );
    }
    if matches!(session_target, SessionTarget::Current) {
        return Err(
            "session_target 'current' needs the conversation the job is created from; this \
             job has none (use 'isolated')"
                .to_string(),
        );
    }
    Ok(())
}

/// Validate a `DeliveryConfig` at creation time.
///
/// `announce` needs `channel` and `to`, and `to` must be in the channel's
/// `allowed_users` (no cross-tenant `to`, #928) unless it is exactly the
/// reply target of the conversation the job is created from: the asker is the
/// recipient. `origin` needs an origin. `proactive` and `none` are not
/// channel-targeted.
pub fn validate_delivery(
    config: &Config,
    delivery: &DeliveryConfig,
    origin: Option<&JobOrigin>,
) -> Result<(), String> {
    let mode = delivery.mode.trim().to_ascii_lowercase();
    if mode == delivery_mode::ORIGIN {
        return check_origin_requirements(&SessionTarget::Isolated, delivery, origin);
    }
    if mode != delivery_mode::ANNOUNCE {
        return Ok(());
    }

    let channel = delivery
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "delivery.channel is required for announce mode".to_string())?;
    let to = delivery
        .to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "delivery.to is required for announce mode".to_string())?;

    if let Some(JobOrigin::Channel {
        channel: origin_channel,
        reply_target,
        ..
    }) = origin
    {
        if channel.eq_ignore_ascii_case(origin_channel) && to == reply_target {
            tracing::debug!(
                channel = %channel,
                "[cron] announce target is the origin reply target; skipping allowed_users check"
            );
            return Ok(());
        }
    }

    // "web" announce is a degenerate case (web has no allowed_users gate).
    if channel.eq_ignore_ascii_case("web") {
        return Ok(());
    }

    match allowed_users_for_channel(config, channel) {
        Some([]) => Ok(()),
        Some(list) => {
            if list.iter().any(|u| u == to) {
                Ok(())
            } else {
                Err(format!(
                    "delivery target '{to}' on channel '{channel}' is not in allowed_users \
                     for that channel; refusing to schedule cross-tenant delivery"
                ))
            }
        }
        None => Err(format!(
            "delivery channel '{channel}' is not configured; cannot validate target"
        )),
    }
}

/// Create an agent job from the current turn.
///
/// `origin` is the conversation to bind the job to (see
/// [`cron::origin::current_job_origin`]). It is stored on the job only when
/// something uses it (`session_target: current` or `delivery: origin`), so an
/// explicitly non-origin job keeps its old trust and routing.
pub fn create_agent_job(
    config: &Config,
    input: AgentJobInput,
    origin: Option<JobOrigin>,
) -> Result<CronJob, String> {
    if cron::limits::enforced(config) && origin.is_none() {
        return Err("cron jobs can only be created from a conversation".to_string());
    }
    let session_target = input.session_target.unwrap_or(if origin.is_some() {
        SessionTarget::Current
    } else {
        SessionTarget::Isolated
    });
    let delivery = input
        .delivery
        .unwrap_or_else(|| default_delivery(origin.as_ref()));

    validate_delivery(config, &delivery, origin.as_ref())?;
    check_origin_requirements(&session_target, &delivery, origin.as_ref())?;

    let uses_origin = matches!(session_target, SessionTarget::Current)
        || delivery
            .mode
            .trim()
            .eq_ignore_ascii_case(delivery_mode::ORIGIN);
    let origin = origin.filter(|_| uses_origin);

    tracing::debug!(
        session_target = session_target.as_str(),
        delivery_mode = %delivery.mode,
        origin = origin.as_ref().map(JobOrigin::kind_str).unwrap_or("none"),
        "[cron] creating agent job"
    );

    let mut spec = AgentJobSpec::new(input.schedule, input.prompt);
    spec.name = input.name;
    spec.session_target = session_target;
    spec.model = input.model;
    spec.delivery = Some(delivery);
    spec.delete_after_run = input.delete_after_run;
    spec.origin = origin;
    cron::add_agent_job_from_spec(config, spec).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "job_builder_tests.rs"]
mod tests;
