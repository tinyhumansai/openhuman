//! The prompt of a `session_target: current` run: a fresh session that is shown
//! a bounded tail of the conversation that created the job, plus a fixed
//! preamble saying the run is unattended.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use {
    crate::cron::channel_bridge, tinyflows_bus::schedule::types::CronJob,
    tinyflows_bus::schedule::types::JobOrigin,
};

/// At most this many trailing messages of the origin conversation.
pub(crate) const CONTEXT_MAX_MESSAGES: usize = 10;
/// At most this many characters across the whole tail; the newest are kept.
pub(crate) const CONTEXT_MAX_CHARS: usize = 1400;

/// Fixed preamble for an unattended scheduled run.
pub(crate) const UNATTENDED_PREAMBLE: &str = "This is a scheduled run, not a live chat. Your final reply is delivered to the user as-is. If there is nothing worth sending, reply exactly NO_REPLY.";

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
        JobOrigin::Channel { history_key, .. } => channel_bridge::history_messages(history_key),
    }
}

/// Prompt for a `current` run: tagged task, unattended preamble, the bounded
/// context tail (when there is one), then the task itself.
pub(crate) fn compose_current_prompt(job_id: &str, name: &str, prompt: &str, tail: &str) -> String {
    let mut out = format!("[cron:{job_id} {name}] {UNATTENDED_PREAMBLE}\n\n");
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
            tracing::debug!(
                job_id = %job.id,
                tail_chars = tail.chars().count(),
                "[cron] composed current-session prompt"
            );
            compose_current_prompt(&job.id, name, &prompt, &tail)
        }
    }
}

#[cfg(test)]
#[path = "origin_context_tests.rs"]
mod tests;
