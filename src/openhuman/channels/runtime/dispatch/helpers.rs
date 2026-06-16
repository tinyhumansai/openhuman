//! Small stateless helpers used by the channel runtime dispatch path.
//!
//! Contains:
//! * [`build_channel_context_block`] — per-turn context injected for non-web
//!   channels.
//! * [`select_acknowledgment_reaction`] — deterministic emoji picker.
//! * [`log_worker_join_result`] / [`spawn_scoped_typing_task`] — worker
//!   lifecycle utilities.
//! * Private [`contains_any`] / [`starts_with_any`] predicates.

use crate::openhuman::channels::context::CHANNEL_TYPING_REFRESH_INTERVAL_SECS;
use crate::openhuman::channels::traits;
use crate::openhuman::channels::Channel;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Maximum characters shown in the debug reply println. Large enough to not truncate
/// real responses while keeping terminal output readable.
pub(super) const REPLY_LOG_TRUNCATE_CHARS: usize = 200;

/// Returns `true` if `s` contains any of the given substrings.
#[inline]
pub(super) fn contains_any(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

/// Returns `true` if `s` starts with any of the given prefixes.
#[inline]
pub(super) fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

/// Build the per-turn `[Channel context]` block prepended to the user
/// message for non-web inbound channels (e.g. Telegram, Discord, Slack).
///
/// Surfaces the active channel and reply target so the model knows
/// where it is talking and can route any tool side-effects (notably
/// `cron_add`) back to the same chat instead of defaulting to the
/// in-app web stream. See issue #928.
///
/// Returns an empty string for web/cli turns (the desktop UI is the
/// default delivery surface, no hint needed).
pub(super) fn build_channel_context_block(msg: &traits::ChannelMessage) -> String {
    let channel = msg.channel.trim();
    if channel.is_empty()
        || channel.eq_ignore_ascii_case("web")
        || channel.eq_ignore_ascii_case("cli")
    {
        return String::new();
    }

    let reply_target = msg.reply_target.trim();
    if reply_target.is_empty() {
        return String::new();
    }

    format!(
        "[Channel context]\n\
         You are responding via the \"{channel}\" channel. Reply target: \"{reply_target}\".\n\
         For any cron/scheduled reminder you create with `cron_add`, set `delivery` to \
         `{{ \"mode\": \"announce\", \"channel\": \"{channel}\", \"to\": \"{reply_target}\" }}` \
         so the reminder is delivered back here instead of the in-app web stream. \
         Only fall back to the default proactive delivery if the user explicitly asks for \
         in-app/desktop notification.\n\n"
    )
}

/// Pick a contextual acknowledgment emoji for an inbound message.
///
/// Intent categories are checked in priority order. Within each category two
/// emoji options are defined; a cheap deterministic index (based on message
/// length + first char value) selects between them so that similar messages
/// don't always produce the identical reaction.
///
/// All emojis used here are in Telegram's standard (non-premium) reaction set.
pub(super) fn select_acknowledgment_reaction(content: &str) -> &'static str {
    let l = content.to_lowercase();

    // Deterministic variant (0 or 1) — avoids true randomness while giving variety.
    let v = content
        .len()
        .wrapping_add(content.chars().next().map_or(0, |c| c as usize))
        & 1;

    let opts: &[&str] = if contains_any(&l, &["thank", "thx", "appreciate", "grateful", "cheers"]) {
        // Gratitude
        &["❤️", "🙏"]
    } else if contains_any(
        &l,
        &[
            "amazing",
            "awesome",
            "incredible",
            "love it",
            "congrat",
            "!!",
        ],
    ) {
        // Excitement / celebration
        &["🔥", "🎉"]
    } else if contains_any(
        &l,
        &[
            "price", "btc", "eth", "crypto", "trade", "pump", "dump", "market", "token", "wallet",
            "defi", "nft", "sol", "bnb",
        ],
    ) {
        // Crypto / finance
        &["💯", "⚡"]
    } else if contains_any(
        &l,
        &[
            "code",
            "function",
            "api",
            "deploy",
            "build",
            "debug",
            "script",
            "git",
            "rust",
            "python",
            "js",
            "typescript",
        ],
    ) {
        // Technical / dev
        &["👨‍💻", "🤓"]
    } else if starts_with_any(
        &l,
        &[
            "hi",
            "hello",
            "hey",
            "sup",
            "good morning",
            "good evening",
            "good afternoon",
        ],
    ) || l == "yo"
        || l.starts_with("yo ")
    {
        // Greeting
        &["🤗", "😁"]
    } else if l.contains('?')
        || starts_with_any(
            &l,
            &[
                "how",
                "what",
                "why",
                "when",
                "where",
                "who",
                "can you",
                "could you",
                "would you",
                "is ",
                "are ",
                "do you",
                "does",
            ],
        )
    {
        // Question / help request
        &["🤔", "✍️"]
    } else {
        // Default — "seen, on it"
        &["👀", "✍️"]
    };

    opts[v % opts.len()]
}

pub(super) fn log_worker_join_result(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::error!("Channel message worker crashed: {error}");
    }
}

pub(super) fn spawn_scoped_typing_task(
    channel: Arc<dyn Channel>,
    recipient: String,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let stop_signal = cancellation_token;
    let refresh_interval = Duration::from_secs(CHANNEL_TYPING_REFRESH_INTERVAL_SECS);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = stop_signal.cancelled() => break,
                _ = tokio::time::sleep(refresh_interval) => {
                    if let Err(e) = channel.start_typing(&recipient).await {
                        tracing::debug!("Failed to start typing on {}: {e}", channel.name());
                    }
                }
            }
        }

        if let Err(e) = channel.stop_typing(&recipient).await {
            tracing::debug!("Failed to stop typing on {}: {e}", channel.name());
        }
    });

    handle
}
