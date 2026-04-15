//! Proactive welcome — fires the welcome agent immediately when the
//! user completes the desktop onboarding wizard, instead of waiting
//! for their first chat message.
//!
//! ## Flow
//!
//! 1. [`crate::openhuman::config::ops::set_onboarding_completed`]
//!    detects a false→true transition and calls [`spawn_proactive_welcome`].
//! 2. That function spawns a detached Tokio task that:
//!    - Pre-builds the JSON snapshot (config + user profile + onboarding
//!      tasks + composio connections) in Rust — no LLM round-trip needed.
//!    - Loads the `welcome` agent via
//!      [`crate::openhuman::agent::Agent::from_config_for_agent`] so
//!      the agent runs with its own `prompt.md`, tool allowlist, and
//!      model hint (`agentic-v1`).
//!    - Calls [`crate::openhuman::agent::Agent::run_single`] with the
//!      pre-built context, skipping iteration 1 (tool calls). The agent
//!      goes straight to writing the personalised welcome message.
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

/// Internal: pre-build context, run the welcome agent, publish the
/// result. Split out from the spawn so it can be unit-tested with
/// an injected Config + mocked provider.
async fn run_proactive_welcome(config: Config) -> anyhow::Result<()> {
    tracing::info!(
        "[welcome::proactive] starting proactive welcome (chat_onboarding_completed={}, ui_onboarding_completed={})",
        config.chat_onboarding_completed,
        config.onboarding_completed
    );

    // Brief delay so the frontend Socket.IO client has time to
    // connect and join the "system" room after the onboarding overlay
    // closes. Without this, the message can arrive before anyone is
    // listening.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // ── Pre-build context in Rust (no LLM round-trip) ────────────
    //
    // Gather the same data the agent would get from calling
    // check_status + composio_list_connections, but directly in Rust.
    // This saves one full LLM iteration (~30-50s).

    let mut snapshot = build_status_snapshot(&config, "flipped");

    // Enrich with user profile + onboarding tasks (best-effort)
    if let serde_json::Value::Object(ref mut map) = snapshot {
        // Onboarding tasks — sync local file read
        match crate::openhuman::app_state::ops::load_stored_app_state(&config) {
            Ok(local_state) => {
                if let Some(tasks) = local_state.onboarding_tasks {
                    map.insert(
                        "onboarding_tasks".to_string(),
                        serde_json::to_value(&tasks).unwrap_or_default(),
                    );
                }
            }
            Err(e) => {
                tracing::warn!("[welcome::proactive] failed to load app state: {e}");
            }
        }

        // User profile — async HTTP, 5s timeout
        match crate::api::jwt::get_session_token(&config) {
            Ok(Some(token)) if !token.is_empty() => {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    crate::openhuman::app_state::ops::fetch_current_user(&config, &token),
                )
                .await
                {
                    Ok(Ok(Some(user))) => {
                        map.insert("user_profile".to_string(), user);
                    }
                    _ => {
                        tracing::debug!("[welcome::proactive] user profile unavailable; omitting");
                    }
                }
            }
            _ => {}
        }

        // Composio connections — async HTTP, 5s timeout
        if let Some(client) = crate::openhuman::composio::client::build_composio_client(&config) {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.list_connections(),
            )
            .await
            {
                Ok(Ok(connections)) => {
                    map.insert(
                        "composio_connections".to_string(),
                        serde_json::to_value(&connections).unwrap_or_default(),
                    );
                }
                _ => {
                    tracing::debug!(
                        "[welcome::proactive] composio connections unavailable; omitting"
                    );
                }
            }
        }
    }

    let snapshot_json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("serialize status snapshot: {e}"))?;

    tracing::debug!(
        snapshot_chars = snapshot_json.len(),
        "[welcome::proactive] pre-built context snapshot"
    );

    // ── Instant draft message (no LLM, appears in ~1-2s) ─────────
    //
    // Publish a short template greeting immediately so the user sees
    // something in the chat while the LLM generates the full welcome.
    let first_name = snapshot
        .get("user_profile")
        .and_then(|u| u.get("firstName"))
        .and_then(|v| v.as_str())
        .unwrap_or("there");

    let draft = format!(
        "Hey {}! Welcome to OpenHuman — give me a sec while I look around your setup...",
        first_name
    );

    publish_global(DomainEvent::ProactiveMessageRequested {
        source: PROACTIVE_WELCOME_SOURCE.to_string(),
        message: draft,
        job_name: Some(PROACTIVE_WELCOME_JOB_NAME.to_string()),
    });

    tracing::info!(
        "[welcome::proactive] instant draft published for user '{}'",
        first_name
    );

    // ── Run the welcome agent (single LLM call) ─────────────────

    let mut agent = Agent::from_config_for_agent(&config, "welcome").map_err(|e| {
        anyhow::anyhow!("build welcome agent: {e} — ensure AgentDefinitionRegistry is initialised")
    })?;
    agent.set_event_context(
        format!("proactive:{PROACTIVE_WELCOME_JOB_NAME}"),
        "proactive",
    );

    // Pre-deliver all context so the agent skips iteration 1 (tool
    // calls) and goes straight to writing the welcome message. This
    // saves one full LLM round-trip. The agent still has tools
    // available for Gmail OAuth (composio_authorize) if needed in
    // subsequent iterations.
    let prompt = format!(
        "[PROACTIVE INVOCATION — the user just finished the desktop onboarding wizard; \
         this is not a reply to anything they typed, it is your opening message.]\n\n\
         Skip iteration 1. The context that `complete_onboarding(check_status)` and \
         `composio_list_connections` would have returned is already provided below. \
         Jump straight to iteration 2 and write the personalised welcome message \
         per your system prompt guidelines.\n\n\
         Status snapshot (treat exactly as if it were the tool return values):\n\
         ```json\n{snapshot_json}\n```\n\n\
         Write your welcome message now."
    );

    tracing::debug!(
        prompt_chars = prompt.len(),
        "[welcome::proactive] invoking welcome agent run_single"
    );

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        agent.run_single(&prompt),
    )
    .await
    .map_err(|_| anyhow::anyhow!("welcome agent timed out after 180s"))?
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
