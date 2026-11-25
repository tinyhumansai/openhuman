//! The prompt of a `session_target: current` run: a fresh session that is shown
//! a bounded tail of the conversation that created the job, plus a fixed
//! preamble saying the run is unattended. Only a job whose prompt opts into
//! silence ([`job_allows_no_reply`]) is offered `NO_REPLY`.

use super::origin_delivery::job_allows_no_reply;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use {
    tinyflows_bus::schedule::types::CronJob, tinyflows_bus::schedule::types::JobOrigin,
    tinyflows_bus::schedule::ChannelSessionIdentity,
};

/// At most this many trailing messages of the origin conversation.
pub(crate) const CONTEXT_MAX_MESSAGES: usize = 10;
/// At most this many characters across the whole tail; the newest are kept.
pub(crate) const CONTEXT_MAX_CHARS: usize = 1400;

/// Preamble for an unattended run that must deliver: the user asked to
/// receive this message.
pub(crate) const DELIVERING_PREAMBLE: &str = "This is a scheduled run, not a live chat. The user asked to receive this message, and your final reply is delivered to them as-is. Always reply with the message itself.";

/// Preamble for an unattended run whose task allows sending nothing.
pub(crate) const CONDITIONAL_PREAMBLE: &str = "This is a scheduled run, not a live chat. Your final reply is delivered to the user as-is. When the task's condition is not met, reply exactly NO_REPLY.";

/// Render the newest messages that fit within `max_messages` / `max_chars` as
/// `role: text` lines, oldest first. A message that does not fit whole is cut
/// to its tail when there is meaningful room left, else dropped.
pub(crate) fn bounded_tail(
    messages: &[(String, String)],
    max_messages: usize,
    max_chars: usize,
) -> String {
    let mut remaining = max_chars;
    let mut picked: Vec<String> = Vec::new();
    for (role, text) in messages.iter().rev().take(max_messages) {
        let text = text.trim();
        if text.is_empty() || !matches!(role.as_str(), "user" | "assistant") {
            continue;
        }
        let line = format!("{role}: {text}");
        let len = line.chars().count();
        // The `\n` that joins this line to the ones already picked counts too.
        let sep = usize::from(!picked.is_empty());
        if len + sep <= remaining {
            remaining -= len + sep;
            picked.push(line);
            continue;
        }
        if remaining >= 80 + sep {
            // One char for the `…` prefix and `sep` for the join newline.
            let keep = remaining - 1 - sep;
            let cut: String = line.chars().skip(len - keep).collect::<String>();
            picked.push(format!("…{cut}"));
        }
        break;
    }
    picked.reverse();
    picked.join("\n")
}

/// The origin conversation's recent messages, oldest first.
async fn origin_messages(
    ctx: &DispatchCtx,
    config: &Config,
    origin: &JobOrigin,
) -> Vec<(String, String)> {
    match origin {
        JobOrigin::Web { thread_id, .. } => {
            match crate::threads::store::blocking::get_messages(
                ctx.clone(),
                config.workspace_dir.clone(),
                thread_id.clone(),
            )
            .await
            {
                Ok(messages) => messages
                    .into_iter()
                    .map(|m| {
                        let role = if m.sender == "user" {
                            "user"
                        } else {
                            "assistant"
                        };
                        (role.to_string(), m.content)
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "[cron] could not read origin thread for context");
                    Vec::new()
                }
            }
        }
        JobOrigin::Channel {
            channel_session: Some(identity),
            channel,
            ..
        } => match validated_channel_session(ctx, config, channel, identity) {
            Ok((session_key, agent_id, tenant)) => {
                let Some(core) = ctx.core() else {
                    tracing::warn!("[cron] channel origin has no resolved core context");
                    return Vec::new();
                };
                let locator = crate::storage::sessions::transcripts_or_files_in(
                    core,
                    &crate::storage::sessions::agent_key_for(&tenant, &agent_id),
                    &config.workspace_dir,
                );
                let session =
                    tinyagents_session::transcript::SessionRef::scoped(session_key, agent_id);
                let head = locator.head_generation(&session);
                locator
                    .read_session_transcript(&head)
                    .and_then(|read| read.read_session().ok().flatten())
                    .map(|history| {
                        history
                            .messages
                            .into_iter()
                            .filter_map(|message| {
                                let role = match message.role.as_str() {
                                    "user" => "user",
                                    "assistant" => "assistant",
                                    _ => return None,
                                };
                                Some((role.to_string(), message.content))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
            Err(error) => {
                tracing::warn!(%error, "[cron] refusing unscoped channel origin context");
                Vec::new()
            }
        },
        JobOrigin::Channel { .. } => Vec::new(),
    }
}

/// Validate persisted channel identity claims against the current callback
/// tenant and the config-owned channel-to-agent selection.
pub(super) fn validated_channel_session(
    ctx: &DispatchCtx,
    config: &Config,
    channel: &str,
    identity: &ChannelSessionIdentity,
) -> Result<(String, String, crate::core::runtime::Tenant), String> {
    let tenant = ctx.tenant().map_err(|error| error.to_string())?;
    let tenant_scope = crate::core::runtime::tenant_key(&tenant, "");
    if identity.tenant_key != tenant_scope {
        return Err("channel origin belongs to a different tenant".into());
    }
    let provider = tinychannels_bus::provider_id(channel);
    let account_id = identity
        .account_id
        .as_deref()
        .filter(|account| !account.trim().is_empty())
        .ok_or_else(|| "channel origin has no configured account identity".to_string())?;
    if identity.provider != provider
        || tinychannels_bus::provider_id(account_id) != provider
        || identity.session_key.trim().is_empty()
        || identity.agent_id.trim().is_empty()
        || identity.conversation.id.trim().is_empty()
    {
        return Err("channel origin identity does not match its trusted route".into());
    }
    let configured_agent = if tenant.profile.is_some() {
        "orchestrator"
    } else {
        config
            .agent
            .channel_agents
            .get(channel)
            .or_else(|| config.agent.channel_agents.get(provider))
            .map(|agent| agent.trim())
            .filter(|agent| !agent.is_empty())
            .unwrap_or("")
    };
    let selected_agent = if configured_agent.is_empty() {
        crate::web_chat::pick_target_agent_id(config)
    } else {
        configured_agent.to_string()
    };
    if selected_agent != identity.agent_id {
        return Err("channel origin agent no longer matches its configured route".into());
    }
    Ok((
        crate::core::runtime::tenant_key(&tenant, &identity.session_key),
        identity.agent_id.clone(),
        tenant,
    ))
}

/// Prompt for a `current` run: tagged task, unattended preamble (the
/// conditional one when `allows_no_reply`), the bounded context tail (when
/// there is one), then the task itself.
pub(crate) fn compose_current_prompt(
    job_id: &str,
    name: &str,
    prompt: &str,
    tail: &str,
    allows_no_reply: bool,
) -> String {
    let preamble = if allows_no_reply {
        CONDITIONAL_PREAMBLE
    } else {
        DELIVERING_PREAMBLE
    };
    let mut out = format!("[cron:{job_id} {name}] {preamble}\n\n");
    if !tail.is_empty() {
        out.push_str("Recent conversation this task was created in (context only):\n");
        out.push_str(tail);
        out.push_str("\n\n");
    }
    out.push_str("Scheduled task: ");
    out.push_str(prompt);
    out
}

/// The prompt a job runs with. `isolated` (and an origin-less job) keeps the
/// plain tagged prompt; `current` and `main` get the context and preamble.
pub(crate) async fn build_run_prompt(
    ctx: &DispatchCtx,
    config: &Config,
    job: &CronJob,
    name: &str,
) -> String {
    let prompt = job.prompt.clone().unwrap_or_default();
    use tinyflows_bus::schedule::types::SessionTarget;
    match (&job.session_target, &job.origin) {
        (SessionTarget::Isolated, _) => format!("[cron:{} {name}] {prompt}", job.id),
        (target, origin) => {
            if matches!(target, SessionTarget::Main) {
                tracing::debug!(
                    job_id = %job.id,
                    "[cron] session_target main is not yet distinct from current; running as current"
                );
            }
            let tail = match origin {
                Some(origin) => {
                    let messages = origin_messages(ctx, config, origin).await;
                    bounded_tail(&messages, CONTEXT_MAX_MESSAGES, CONTEXT_MAX_CHARS)
                }
                None => String::new(),
            };
            let allows_no_reply = job_allows_no_reply(job);
            tracing::debug!(
                job_id = %job.id,
                tail_chars = tail.chars().count(),
                allows_no_reply,
                "[cron] composed current-session prompt"
            );
            compose_current_prompt(&job.id, name, &prompt, &tail, allows_no_reply)
        }
    }
}

#[cfg(test)]
#[path = "origin_context_tests.rs"]
mod tests;
