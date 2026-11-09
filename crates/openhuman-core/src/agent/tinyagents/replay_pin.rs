//! The history-replay options a thread starts with, and keeps.
//!
//! Expiring old tool results and leaving earlier turns' reasoning out of a
//! request both change the bytes of every later prompt (the provider's cached
//! prefix), so a session decides them once, when it creates the thread, and
//! records them on the thread's first transcript row beside the discovery
//! options (`discovery/pin.rs` is the model). A resumed thread reads them back;
//! the config and the embedder's options only choose them for the next thread.
//!
//! The compaction tail (`[context] keep_recent_tokens`) is pinned with them,
//! but only when the thread started with one: a thread that pinned none follows
//! the config, as every thread did before.
//!
//! A compaction carries the record onto the first row of the generation it
//! opens ([`carry_forward`]), so a thread keeps its options after the row that
//! held them is folded away.
//!
//! A thread that records nothing (every thread written before this existed,
//! and any thread created with today's behaviour) resolves to
//! [`ReplayOptions::TODAY`] whatever the config says now.
//!
//! The options reach the harness per run as
//! [`ExpiryOverrides`] on `RunConfig::result_expiry` and as
//! `RunConfig::replay_reasoning`, every field set, so the harness-wide policy
//! never leaks into a session turn.

use serde::{Deserialize, Serialize};
use tinyagents_harness::reasoning_replay::ReasoningReplay;
use tinyagents_harness::result_expiry::{ExpiryOverrides, DEFAULT_EXPIRY_MAX_AGE_TURNS};
use tinyagents_session::transcript::TranscriptMessage;

use super::discovery::{record_metadata, recorded_metadata};
use crate::config::ContextSettings;

/// Key under a transcript row's `extra_metadata` that holds the recorded
/// options.
pub const REPLAY_METADATA_KEY: &str = "context_replay";

/// The history-replay options of one thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayOptions {
    /// A tool result with more than this many user turns after it is replaced
    /// by a one-line note. `None` keeps every result.
    pub tool_result_max_age_turns: Option<u32>,
    /// Tools whose results never expire. Empty while expiry is off.
    pub tool_result_exempt_tools: Vec<String>,
    /// Which turns' reasoning a request carries.
    pub replay_reasoning: ReasoningReplay,
    /// The verbatim tail, in tokens, an in-turn compaction keeps after its
    /// summary. `None` is not pinned: the config's value applies, as it does
    /// for a thread that recorded nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_recent_tokens: Option<u32>,
}

impl ReplayOptions {
    /// Today's behaviour: the harness defaults, and what a thread that
    /// recorded nothing has always run with.
    pub const TODAY: Self = Self {
        tool_result_max_age_turns: None,
        tool_result_exempt_tools: Vec::new(),
        replay_reasoning: ReasoningReplay::All,
        keep_recent_tokens: None,
    };

    /// The options `[context]` asks new threads to start with. An age of `0`
    /// means off, and an exempt list means nothing without an age, so both
    /// normalize to what [`Self::TODAY`] holds.
    pub fn from_config(config: &ContextSettings) -> Self {
        let age = config.tool_result_max_age_turns.filter(|age| *age > 0);
        Self {
            tool_result_max_age_turns: age,
            tool_result_exempt_tools: if age.is_some() {
                config.tool_result_exempt_tools.clone()
            } else {
                Vec::new()
            },
            replay_reasoning: config.replay_reasoning.into(),
            keep_recent_tokens: config.keep_recent_tokens,
        }
    }

    /// The compaction tail a turn on this thread keeps: the pinned one, or
    /// `configured` while the thread pinned none.
    pub fn keep_recent_tokens_or(&self, configured: Option<u64>) -> Option<u64> {
        self.keep_recent_tokens.map(u64::from).or(configured)
    }

    /// The per-run overrides that make the harness expire results exactly as
    /// these options say.
    pub fn expiry_overrides(&self) -> ExpiryOverrides {
        ExpiryOverrides {
            enabled: Some(self.tool_result_max_age_turns.is_some()),
            max_age_turns: Some(
                self.tool_result_max_age_turns
                    .map_or(DEFAULT_EXPIRY_MAX_AGE_TURNS, |age| age as usize),
            ),
            exempt_tools: Some(self.tool_result_exempt_tools.clone()),
        }
    }

    /// The options a thread recorded on its transcript `rows`, if any.
    pub fn recorded_in(rows: &[TranscriptMessage]) -> Option<Self> {
        recorded_metadata(rows, REPLAY_METADATA_KEY)
    }

    /// Records these options on the first fresh row of a transcript being
    /// written, returning whether one was found. A row already stored is never
    /// touched: rewriting it would make the commit open a new generation.
    pub fn record_on(&self, rows: &mut [TranscriptMessage], fresh: &[bool]) -> bool {
        let value = serde_json::to_value(self).expect("plain options serialize");
        record_metadata(rows, fresh, REPLAY_METADATA_KEY, value)
    }
}

/// Carries the options recorded on `prior` onto the first of `rows`, a
/// transcript about to open a compacted generation, when `rows` lost them.
/// Returns whether it recorded them. See `discovery::carry_forward`: the
/// record lives on a row, and a refreshed system prompt or a summary standing
/// in for the folded rows drops it. The generation is new, so writing onto its
/// first row rewrites no stored row.
pub fn carry_forward(prior: &[TranscriptMessage], rows: &mut [TranscriptMessage]) -> bool {
    if ReplayOptions::recorded_in(rows).is_some() {
        return false;
    }
    let Some(options) = ReplayOptions::recorded_in(prior) else {
        return false;
    };
    options.record_on(rows, &[true])
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self::TODAY
    }
}

/// What a turn runs with, and what it must write down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedReplay {
    /// The options this turn runs with.
    pub effective: ReplayOptions,
    /// The options to record on the thread, set only for the turn that starts
    /// it and only when they differ from [`ReplayOptions::TODAY`].
    pub record: Option<ReplayOptions>,
}

/// Decides a session turn's options.
///
/// `generation_start` is true for the turn that prepares the thread's first
/// prompt: it takes the options `config` asks for and records them, unless
/// `rows` already record some (a compacted head whose prefix is prepared
/// again keeps its pin). Every other turn reads what `rows` recorded, or
/// [`ReplayOptions::TODAY`].
pub fn resolve(
    config: &ContextSettings,
    rows: &[TranscriptMessage],
    generation_start: bool,
) -> ResolvedReplay {
    let recorded = ReplayOptions::recorded_in(rows);
    if generation_start && recorded.is_none() {
        let effective = ReplayOptions::from_config(config);
        let record = (effective != ReplayOptions::TODAY).then(|| effective.clone());
        return ResolvedReplay { effective, record };
    }
    ResolvedReplay {
        effective: recorded.unwrap_or(ReplayOptions::TODAY),
        record: None,
    }
}

#[cfg(test)]
#[path = "replay_pin_tests.rs"]
mod tests;
