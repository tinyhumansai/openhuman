//! Proactive welcome — fires the welcome agent immediately when the
//! user completes the desktop onboarding wizard, instead of waiting
//! for their first chat message.
//!
//! ## Flow
//!
//! 1. [`crate::openhuman::config::ops::set_onboarding_completed`]
//!    detects a false→true transition and calls [`spawn_proactive_welcome`].
//! 2. That function spawns a detached Tokio task that:
//!    - Publishes **Template 1** immediately (t ≈ 0 ms): a time-of-day
//!      greeting that names any channels the user already has connected.
//!      This appears in the chat bubble within milliseconds of the
//!      wizard closing.
//!    - Simultaneously starts welcome-agent LLM inference (see below).
//!    - After 4 seconds publishes **Template 2**: "Getting everything
//!      ready for you…" — a loading indicator while inference continues.
//!    - When inference finishes publishes the full personalised welcome.
//!
//! ### Welcome agent inference (parallel path)
//!
//!    - Builds the same JSON status snapshot that
//!      `complete_onboarding` `check_status` would return (`pending`,
//!      `ready_to_complete: false` until the user has conversed or
//!      connected Composio).
//!    - Loads the `welcome` agent via
//!      [`crate::openhuman::agent::Agent::from_config_for_agent`] so
//!      the agent runs with its own `prompt.md`, tool allowlist, and
//!      model hint.
//!    - Calls [`crate::openhuman::agent::Agent::run_single`] with a
//!      prompt that embeds the snapshot, skips the tool-call iteration,
//!      and instructs the agent not to repeat the greeting (templates
//!      already delivered that).
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
use crate::openhuman::tools::implementations::agent::onboarding_status::build_status_snapshot;

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

// ---------------------------------------------------------------------------
// Template helpers
// ---------------------------------------------------------------------------

/// Returns a time-of-day greeting string based on the current UTC hour.
///
/// Uses the machine local clock so greetings better match user
/// expectations in desktop-first usage. Defaults to "Good afternoon"
/// if the system clock is unavailable.
fn time_of_day_greeting() -> &'static str {
    let hour = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|_| {
            chrono::Local::now()
                .format("%H")
                .to_string()
                .parse::<u8>()
                .ok()
        })
        .unwrap_or(14); // default to afternoon on clock error
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Hey there",
    }
}

/// Build Template 1 — an immediate personalised greeting that names any
/// channels the user already has connected.
///
/// Shown instantly (t ≈ 0 ms) while LLM inference runs in parallel.
/// Receives the JSON status snapshot so it can reference real setup data
/// without a second config read.
fn build_template_greeting(snapshot: &serde_json::Value) -> String {
    let greeting = time_of_day_greeting();

    let channels: Vec<&str> = snapshot
        .get("channels_connected")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if channels.is_empty() {
        return format!("{greeting}! Getting your workspace ready.");
    }

    let channel_list = if channels.len() == 1 {
        channels[0].to_string()
    } else if channels.len() == 2 {
        format!("{} and {}", channels[0], channels[1])
    } else {
        // 3+ channels: "a, b, and c"
        let (last, rest) = channels
            .split_last()
            .expect("len >= 3 guaranteed by else branch");
        format!("{}, and {last}", rest.join(", "))
    };

    format!(
        "{greeting}! I can see you've got {channel_list} connected \
         — pulling your full setup together now."
    )
}

// ---------------------------------------------------------------------------
// Core proactive flow
// ---------------------------------------------------------------------------

/// Internal: build the snapshot, fire templates, run the welcome agent in
/// parallel with the template delays, and publish all three messages in
/// order.
///
/// Split out from the spawn so it can be unit-tested with an injected
/// Config.
async fn run_proactive_welcome(config: Config) -> anyhow::Result<()> {
    tracing::info!(
        "[welcome::proactive] starting proactive welcome \
         (chat_onboarding_completed={}, ui_onboarding_completed={})",
        config.chat_onboarding_completed,
        config.onboarding_completed
    );

    // `chat_onboarding_completed` is still `false` at this point —
    // `set_onboarding_completed` no longer pre-flips it. The flag
    // only flips when the welcome agent calls `complete_onboarding`
    // after meeting the engagement criteria. Proactive welcome is
    // the opening greeting — no exchanges yet, no snapshot-driven
    // toolkit/webview personalisation. Pass empty lists so the
    // snapshot shape matches `check_onboarding_status` without
    // paying for the Composio/cookie lookup here.
    let snapshot = build_status_snapshot(
        &config,
        "pending",
        0,
        false,
        "fewer_than_min_exchanges_and_no_skills_connected",
        &[],
        serde_json::Value::Object(Default::default()),
    );
    let snapshot_json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("serialize status snapshot: {e}"))?;
    tracing::debug!(
        snapshot_chars = snapshot_json.len(),
        "[welcome::proactive] built status snapshot"
    );

    // --- Template 1: immediate greeting (t ≈ 0 ms) --------------------
    let template_greeting = build_template_greeting(&snapshot);
    tracing::debug!("[welcome::proactive] publishing template 1 (immediate greeting)");
    publish_global(DomainEvent::ProactiveMessageRequested {
        source: PROACTIVE_WELCOME_SOURCE.to_string(),
        message: template_greeting,
        job_name: Some(PROACTIVE_WELCOME_JOB_NAME.to_string()),
    });

    // --- Build agent and prompt ----------------------------------------
    let mut agent = Agent::from_config_for_agent(&config, "welcome").map_err(|e| {
        anyhow::anyhow!("build welcome agent: {e} — ensure AgentDefinitionRegistry is initialised")
    })?;
    agent.set_event_context(
        format!("proactive:{PROACTIVE_WELCOME_JOB_NAME}"),
        "proactive",
    );

    // The reactive welcome prompt asks for visible prose plus
    // `check_onboarding_status` on the first iteration. Here we
    // pre-inject the snapshot so the model can write the long welcome
    // without a tool round-trip. If it calls `check_onboarding_status`
    // anyway, the result is consistent: pending, not ready to complete.
    //
    // We also tell the agent that two greeting template messages have
    // already been shown so it does not open with a redundant "Good
    // morning / Hey there" — the personalised setup summary should
    // start immediately.
    let prompt = format!(
        "[PROACTIVE — the user just finished the desktop onboarding wizard. This is your \
         opening message, not a reply.]\n\n\
         [Two template messages already appeared before your turn: a time-of-day greeting \
         and \"Getting everything ready for you...\" — so DO NOT open with \"hey\", \
         \"good morning\", \"hi\", or any greeting. Jump straight into the personalised bit.]\n\n\
         **Voice: long-lost friend.** Warm, familiar, a little excited to see them, like you're \
         picking up a thread. Not formal, not a host welcoming a guest. If a `### PROFILE.md` \
         block is in your system prompt, USE IT — reference one specific thing about them \
         (their work, interests, something they're into) the way a friend would mention \
         it, not the way a CRM would log it. Do not surface location or other sensitive \
         profile details unless the user already brought them up. No PROFILE.md? Skip the \
         personal bit, stay casual.\n\n\
         Lowercase fine. No corporate language, no \"I'm OpenHuman\", no feature pitch. Short. \
         Then nudge once toward connecting Gmail (only if not already connected per the snapshot \
         you fetch) — phrased as a question, not a sell.\n\n\
         **Make exactly one tool call: `check_onboarding_status` with no args.** Use the result \
         to know what's connected before you write. Do NOT call `complete_onboarding` — the \
         user has not had any conversation yet. Do NOT call any other tool.\n\n\
         A pre-built snapshot is below for context (treat as a hint; the live tool call is \
         the source of truth):\n\
         ```json\n{snapshot_json}\n```\n\n\
         After the tool call, output STRICT JSON only as your final assistant message:\n\
         {{\"messages\":[\"<m1>\",\"<m2>\"]}}\n\
         - 2 messages. Maximum 3.\n\
         - Each message: 1-2 short sentences. No markdown headings, no bullet lists.\n\
         - No code fences, no text outside the JSON."
    );
    tracing::debug!(
        prompt_chars = prompt.len(),
        "[welcome::proactive] invoking welcome agent run_single (parallel with template delay)"
    );

    // --- Run LLM and 4 s template delay concurrently ------------------
    //
    // Template 2 fires after 4 seconds regardless of LLM speed, giving
    // the user visible feedback during inference. The LLM response is
    // published only after inference completes (which is typically
    // 10–30 s for a 200-350 word welcome).
    //
    // `tokio::join!` drives both futures on the current task, so no
    // additional Send bounds are needed on `Agent`.
    let (llm_result, ()) = tokio::join!(agent.run_single(&prompt), async {
        tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
        tracing::debug!("[welcome::proactive] publishing template 2 (loading indicator)");
        publish_global(DomainEvent::ProactiveMessageRequested {
            source: PROACTIVE_WELCOME_SOURCE.to_string(),
            message: "Getting everything ready for you...".to_string(),
            job_name: Some(PROACTIVE_WELCOME_JOB_NAME.to_string()),
        });
    });

    let response =
        llm_result.map_err(|e| anyhow::anyhow!("welcome agent run_single failed: {e}"))?;

    let trimmed = response.trim();
    // Tolerate common fenced JSON wrappers from the model:
    // ```json
    // { ... }
    // ```
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(str::trim_start)
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    if trimmed.is_empty() {
        anyhow::bail!("welcome agent returned empty response");
    }

    #[derive(serde::Deserialize)]
    struct ProactiveMessagesPayload {
        messages: Vec<String>,
    }

    // Preferred path: strict JSON payload from the model.
    // Fallback path: split freeform prose into paragraph-ish chunks so users
    // still receive multiple chat bubbles even if the provider drifts format.
    let messages: Vec<String> = match serde_json::from_str::<ProactiveMessagesPayload>(trimmed) {
        Ok(payload) => payload
            .messages
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect(),
        Err(parse_err) => {
            tracing::warn!(
                error = %parse_err,
                "[welcome::proactive] response was not valid JSON payload; falling back to paragraph splitting"
            );
            trimmed
                .split("\n\n")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        }
    };

    if messages.is_empty() {
        anyhow::bail!("welcome agent returned no publishable messages");
    }

    tracing::info!(
        message_count = messages.len(),
        response_chars = trimmed.chars().count(),
        "[welcome::proactive] welcome agent produced multi-message payload — publishing ProactiveMessageRequested"
    );

    // --- Publish LLM responses (messages 3+) --------------------------
    for (idx, message) in messages.iter().enumerate() {
        if idx > 0 {
            // Slight pacing so bubbles appear progressively instead of as a wall.
            let pace_ms = (message.chars().count() as u64).clamp(600, 1200);
            tokio::time::sleep(tokio::time::Duration::from_millis(pace_ms)).await;
        }
        publish_global(DomainEvent::ProactiveMessageRequested {
            source: PROACTIVE_WELCOME_SOURCE.to_string(),
            message: message.clone(),
            job_name: Some(PROACTIVE_WELCOME_JOB_NAME.to_string()),
        });
    }

    // Post-publish confirmation so the log clearly marks end-to-end
    // success (publish_global is fire-and-forget; without this line,
    // "reached the end cleanly" is ambiguous from the log alone).
    tracing::debug!(
        source = PROACTIVE_WELCOME_SOURCE,
        job_name = PROACTIVE_WELCOME_JOB_NAME,
        published_llm_messages = messages.len(),
        "[welcome::proactive] proactive welcome flow complete \
         (template messages + multi-part llm response published)"
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

    #[test]
    fn template_greeting_no_channels() {
        let snapshot = serde_json::json!({ "channels_connected": [] });
        let msg = build_template_greeting(&snapshot);
        assert!(
            msg.contains("workspace"),
            "expected workspace mention: {msg}"
        );
        // No channel list when there are none
        assert!(
            !msg.contains("connected —"),
            "unexpected channel suffix: {msg}"
        );
    }

    #[test]
    fn template_greeting_one_channel() {
        let snapshot = serde_json::json!({ "channels_connected": ["telegram"] });
        let msg = build_template_greeting(&snapshot);
        assert!(msg.contains("telegram"), "expected channel name: {msg}");
        assert!(msg.contains("connected"), "expected 'connected': {msg}");
    }

    #[test]
    fn template_greeting_two_channels() {
        let snapshot = serde_json::json!({ "channels_connected": ["telegram", "discord"] });
        let msg = build_template_greeting(&snapshot);
        assert!(msg.contains("telegram"), "expected telegram: {msg}");
        assert!(msg.contains("discord"), "expected discord: {msg}");
        assert!(msg.contains(" and "), "expected 'and' join: {msg}");
        // Should not have serial comma for exactly two items
        assert!(
            !msg.contains(", and"),
            "unexpected serial comma for two items: {msg}"
        );
    }

    #[test]
    fn template_greeting_three_channels() {
        let snapshot = serde_json::json!({
            "channels_connected": ["telegram", "discord", "slack"]
        });
        let msg = build_template_greeting(&snapshot);
        assert!(msg.contains("telegram"), "{msg}");
        assert!(msg.contains("discord"), "{msg}");
        assert!(msg.contains("slack"), "{msg}");
        // Oxford comma for 3+ items
        assert!(
            msg.contains(", and "),
            "expected Oxford comma for 3 items: {msg}"
        );
    }

    #[test]
    fn template_greeting_missing_channels_key() {
        // Snapshot without the key should not panic and should return a
        // non-empty fallback greeting.
        let snapshot = serde_json::json!({});
        let msg = build_template_greeting(&snapshot);
        assert!(!msg.is_empty(), "expected non-empty fallback message");
        assert!(
            msg.contains("workspace"),
            "expected workspace fallback: {msg}"
        );
    }

    #[test]
    fn time_of_day_greeting_returns_known_string() {
        let greeting = time_of_day_greeting();
        let valid = [
            "Good morning",
            "Good afternoon",
            "Good evening",
            "Hey there",
        ];
        assert!(
            valid.contains(&greeting),
            "unexpected greeting string: {greeting}"
        );
    }
}
