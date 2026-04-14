//! Proactive welcome — fires the welcome agent immediately when the
//! user completes the desktop onboarding wizard, instead of waiting
//! for their first chat message.
//!
//! ## Flow
//!
//! 1. [`crate::openhuman::config::ops::set_onboarding_completed`]
//!    detects a false→true transition and calls [`spawn_proactive_welcome`].
//! 2. That function spawns a detached Tokio task that:
//!    - Builds the same JSON status snapshot
//!      `tools::implementations::agent::complete_onboarding`'s
//!      `check_status` would have returned, with `finalize_action =
//!      "flipped"` (the caller has already flipped
//!      `chat_onboarding_completed`).
//!    - Loads the `welcome` agent via
//!      [`crate::openhuman::agent::Agent::from_config_for_agent`] so
//!      the agent runs with its own `prompt.md`, tool allowlist, and
//!      model hint.
//!    - Calls [`crate::openhuman::agent::Agent::run_single`] with a
//!      prompt that embeds the snapshot and instructs the agent to
//!      skip iteration 1 (the tool call) and go straight to iteration
//!      2 (writing the welcome message). Because we already flipped
//!      `chat_onboarding_completed`, the `complete_onboarding` tool
//!      would be a no-op anyway — but avoiding the extra round-trip
//!      keeps latency down.
//!    - On success, publishes
//!      [`DomainEvent::ProactiveMessageRequested`] so the existing
//!      [`crate::openhuman::channels::proactive::ProactiveMessageSubscriber`]
//!      delivers the message to the web channel (and any active
//!      external channel) without any new transport code.
//!
//! All steps log at `debug` / `info` so operators can trace the
//! proactive welcome end-to-end: `[welcome::proactive] ...`.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::tools::implementations::agent::complete_onboarding::build_status_snapshot;

/// Event-bus `source` label attached to the proactive welcome message.
/// Kept as a constant so tests and channel-side filters have a stable
/// grep target.
pub const PROACTIVE_WELCOME_SOURCE: &str = "onboarding_completed";

/// Job name used when publishing [`DomainEvent::ProactiveMessageRequested`].
/// Matches the cron-job naming convention so
/// [`crate::openhuman::channels::proactive::ProactiveMessageSubscriber`]
/// routes it under `proactive:welcome`.
pub const PROACTIVE_WELCOME_JOB_NAME: &str = "welcome";

/// Fire-and-forget launch of the welcome agent after onboarding
/// completes.
///
/// Spawned on a detached Tokio task so the caller's RPC response
/// path is never blocked. Failures are logged at `warn` and
/// swallowed — the welcome is best-effort, and the user can still
/// get a (less-polished) welcome by sending their first message
/// (which would route through the normal dispatch path, since the
/// caller flips `chat_onboarding_completed` before invoking us).
pub fn spawn_proactive_welcome(config: Config) {
    tokio::spawn(async move {
        if let Err(e) = run_proactive_welcome(config).await {
            tracing::warn!(
                error = %e,
                "[welcome::proactive] failed to deliver proactive welcome — \
                 falling back to on-first-message flow"
            );
        }
    });
}

/// Internal: build the snapshot, run the welcome agent, publish the
/// result. Split out from the spawn so it can be unit-tested with
/// an injected Config + mocked provider.
async fn run_proactive_welcome(config: Config) -> anyhow::Result<()> {
    tracing::info!(
        "[welcome::proactive] starting proactive welcome (chat_onboarding_completed={}, ui_onboarding_completed={})",
        config.chat_onboarding_completed,
        config.onboarding_completed
    );

    // The caller (set_onboarding_completed) already flipped
    // `chat_onboarding_completed`, so the snapshot always reports
    // `"flipped"` — matches the state the welcome agent's prompt.md
    // treats as "first run, authenticated, deliver the full welcome".
    let snapshot = build_status_snapshot(&config, "flipped");
    let snapshot_json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("serialize status snapshot: {e}"))?;
    tracing::debug!(
        snapshot_chars = snapshot_json.len(),
        "[welcome::proactive] built status snapshot"
    );

    let mut agent = Agent::from_config_for_agent(&config, "welcome").map_err(|e| {
        anyhow::anyhow!("build welcome agent: {e} — ensure AgentDefinitionRegistry is initialised")
    })?;
    agent.set_event_context(
        format!("proactive:{PROACTIVE_WELCOME_JOB_NAME}"),
        "proactive",
    );

    // The welcome prompt.md is insistent that iteration 1 must be a
    // `complete_onboarding` tool call. We bypass that here by
    // pre-delivering the snapshot inside the user message and
    // explicitly overriding iteration 1. Capable models comply; if a
    // model calls the tool anyway, the tool returns
    // `finalize_action: "already_complete"` (we've pre-flipped the
    // flag) so the message still lands — just with a slightly
    // different framing.
    let prompt = format!(
        "[PROACTIVE INVOCATION — the user just finished the desktop onboarding wizard; \
         this is not a reply to anything they typed, it is your opening message.]\n\n\
         Skip iteration 1. Do NOT call `complete_onboarding` or any other tool. The \
         status snapshot that `complete_onboarding(check_status)` would have returned \
         is already provided below. Jump straight to iteration 2 and write the \
         personalised welcome message per your system prompt guidelines.\n\n\
         Status snapshot (treat exactly as if it were the tool return value):\n\
         ```json\n{snapshot_json}\n```\n\n\
         Write iteration 2 now."
    );
    tracing::debug!(
        prompt_chars = prompt.len(),
        "[welcome::proactive] invoking welcome agent run_single"
    );

    let response = agent
        .run_single(&prompt)
        .await
        .map_err(|e| anyhow::anyhow!("welcome agent run_single failed: {e}"))?;

    let trimmed = response.trim();
    if trimmed.is_empty() {
        anyhow::bail!("welcome agent returned empty response");
    }

    tracing::info!(
        response_chars = trimmed.chars().count(),
        "[welcome::proactive] welcome agent produced message — publishing ProactiveMessageRequested"
    );

    publish_global(DomainEvent::ProactiveMessageRequested {
        source: PROACTIVE_WELCOME_SOURCE.to_string(),
        message: trimmed.to_string(),
        job_name: Some(PROACTIVE_WELCOME_JOB_NAME.to_string()),
    });

    // Post-publish confirmation. `publish_global` is a best-effort
    // broadcast send that swallows lag / no-subscriber errors, so
    // without this line the caller can't distinguish "reached the
    // end successfully" from "silently bailed somewhere above" by
    // reading the log alone.
    tracing::debug!(
        source = PROACTIVE_WELCOME_SOURCE,
        job_name = PROACTIVE_WELCOME_JOB_NAME,
        response_chars = trimmed.chars().count(),
        "[welcome::proactive] proactive welcome flow complete"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_job_name_constants_are_stable() {
        // These strings show up in channel-side filters and logs — a
        // silent rename would break downstream grep-based traces.
        assert_eq!(PROACTIVE_WELCOME_SOURCE, "onboarding_completed");
        assert_eq!(PROACTIVE_WELCOME_JOB_NAME, "welcome");
    }
}
