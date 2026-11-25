//! The discovery options a thread starts with, and keeps.
//!
//! Four `tool_search` options change what a thread's prompt looks like:
//! whether found tools join the request's `tools` list, whether a result is
//! printed on one line, whether a used result is collapsed afterwards, and
//! whether a `tool_call` wrapper is repaired. Switching one under a thread
//! that is already running changes the bytes of every later prompt (the
//! provider's cached prefix) and what the model has learned to imitate, so a
//! session decides them once, when it creates the thread, and records them on
//! the thread's first transcript row. A resumed thread reads them back; the
//! config and the embedder's options only choose them for the next thread.
//!
//! A host can force new options onto a thread that exists
//! (`ToolSearchConfig::force_pin`): the turn records them on a later row and
//! the latest record wins. A compaction carries the record onto the first row
//! of the generation it opens ([`carry_forward`]), or onto its last row when a
//! retained row holds an older record.
//!
//! A thread that records nothing (every thread written before this existed,
//! and any thread created with today's behaviour) resolves to
//! [`DiscoveryOptions::TODAY`], the harness defaults, whatever the config says
//! now.
//!
//! The options reach the harness per run as
//! [`DiscoveryOverrides`](tinyagents_harness::tool::discover::DiscoveryOverrides)
//! on `RunConfig::discovery`, every field set, so the process-wide policy
//! never leaks into a session turn.

use serde::{Deserialize, Serialize};
use tinyagents_harness::tool::discover::DiscoveryOverrides;
use tinyagents_session::transcript::TranscriptMessage;

use crate::config::schema::ToolSearchConfig;

/// Key under a transcript row's `extra_metadata` that holds the recorded
/// options.
pub const DISCOVERY_METADATA_KEY: &str = "tool_discovery";

/// The prompt-shaping `tool_search` options of one thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryOptions {
    /// A successful search adds its matches to the request's `tools` list.
    pub promote_found_tools: bool,
    /// A search result's JSON is printed on one line.
    pub compact_search_results: bool,
    /// A search result is rewritten, once its run ends, to the schemas of the
    /// tools the run called.
    pub collapse_search_results: bool,
    /// A call buried in a `tool_call`-style wrapper is retargeted at the real
    /// tool it names.
    pub repair_tool_call_wrapper: bool,
}

impl DiscoveryOptions {
    /// Today's behaviour: the harness defaults, and what a thread that
    /// recorded nothing has always run with.
    pub const TODAY: Self = Self {
        promote_found_tools: true,
        compact_search_results: false,
        collapse_search_results: false,
        repair_tool_call_wrapper: true,
    };

    /// The options `[agent].tool_search` asks new threads to start with.
    pub fn from_config(config: &ToolSearchConfig) -> Self {
        Self {
            promote_found_tools: config.promote_found_tools,
            compact_search_results: config.compact_search_results,
            collapse_search_results: config.collapse_search_results,
            repair_tool_call_wrapper: config.repair_tool_call_wrapper,
        }
    }

    /// Writes these options into `config`, where a session reads them from.
    pub fn apply_to(&self, config: &mut ToolSearchConfig) {
        config.promote_found_tools = self.promote_found_tools;
        config.compact_search_results = self.compact_search_results;
        config.collapse_search_results = self.collapse_search_results;
        config.repair_tool_call_wrapper = self.repair_tool_call_wrapper;
    }

    /// The per-run overrides that make the harness use exactly these options.
    pub fn overrides(&self) -> DiscoveryOverrides {
        DiscoveryOverrides {
            promote_found_tools: Some(self.promote_found_tools),
            compact_search_results: Some(self.compact_search_results),
            collapse_search_results: Some(self.collapse_search_results),
            repair_tool_call_wrapper: Some(self.repair_tool_call_wrapper),
        }
    }

    /// The options a thread recorded on its transcript `rows`, if any.
    pub fn recorded_in(rows: &[TranscriptMessage]) -> Option<Self> {
        recorded_metadata(rows, DISCOVERY_METADATA_KEY)
    }

    /// Records these options on the first fresh row of a transcript being
    /// written, returning whether one was found. A row already in the
    /// transcript is never touched: rewriting it would make the commit open a
    /// new generation instead of extending the thread.
    pub fn record_on(&self, rows: &mut [TranscriptMessage], fresh: &[bool]) -> bool {
        let value = serde_json::to_value(self).expect("plain booleans serialize");
        record_metadata(rows, fresh, DISCOVERY_METADATA_KEY, value)
    }
}

/// The value a thread recorded under `key` on its transcript `rows`, if any.
/// The latest record wins: a host that forces new options onto a thread
/// records them on a later row, since a stored row is never rewritten.
pub(crate) fn recorded_metadata<T: serde::de::DeserializeOwned>(
    rows: &[TranscriptMessage],
    key: &str,
) -> Option<T> {
    rows.iter()
        .rev()
        .filter_map(|row| row.extra_metadata.as_ref())
        .find_map(|metadata| metadata.get(key))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Writes `value` under `key` in the `extra_metadata` of the first fresh row,
/// returning whether one was found. Other keys on the row are kept, so
/// several pins share the thread's first row.
pub(crate) fn record_metadata(
    rows: &mut [TranscriptMessage],
    fresh: &[bool],
    key: &str,
    value: serde_json::Value,
) -> bool {
    let Some(index) = fresh.iter().position(|is_fresh| *is_fresh) else {
        return false;
    };
    let Some(row) = rows.get_mut(index) else {
        return false;
    };
    match row
        .extra_metadata
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
    {
        serde_json::Value::Object(map) => {
            map.insert(key.to_string(), value);
            true
        }
        _ => false,
    }
}

/// Carries the latest options recorded on `prior`, or the `forced` ones this
/// turn records, onto `rows`, a transcript about to open a compacted
/// generation, when `rows` does not end up with them as its latest record.
/// Returns whether it recorded them.
///
/// A compaction writes the retained rows into a fresh generation. The record
/// lives on a row, so it survives only if that row is retained unchanged; a
/// refreshed system prompt, or a summary standing in for the folded rows,
/// drops it, and the thread would fall back to today's behaviour. Writing it
/// onto the first row is free here: the generation is new, so no stored row
/// is rewritten.
///
/// A retained row can also hold an *older* record while a later row, folded
/// away, held the latest: a host that forced new options recorded them on a
/// later row, and the latest record wins. The latest options are then written
/// onto the last row, after every retained one, so the older record cannot win
/// again. `forced`, the options this turn records on its first fresh row, takes
/// precedence over `prior`, since a fresh row can precede a retained one.
/// `rows` that merely extend `prior` are left alone.
pub fn carry_forward(
    prior: &[TranscriptMessage],
    rows: &mut [TranscriptMessage],
    forced: Option<&DiscoveryOptions>,
) -> bool {
    // A turn that extends the stored rows appends; it never needs a record
    // carried, and writing one onto a stored row would rewrite it, so the
    // commit would open a new generation instead of extending the thread.
    if rows.len() >= prior.len() && rows[..prior.len()] == *prior {
        return false;
    }
    let Some(latest) = forced
        .copied()
        .or_else(|| DiscoveryOptions::recorded_in(prior))
    else {
        return false;
    };
    match DiscoveryOptions::recorded_in(rows) {
        Some(held) if held == latest => false,
        Some(_) => {
            let mut last = vec![false; rows.len()];
            if let Some(slot) = last.last_mut() {
                *slot = true;
            }
            latest.record_on(rows, &last)
        }
        None => latest.record_on(rows, &[true]),
    }
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self::TODAY
    }
}

/// What a turn runs with, and what it must write down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedDiscovery {
    /// The options this turn runs with.
    pub effective: DiscoveryOptions,
    /// The options to record on the thread, set only for the turn that starts
    /// it and only when they differ from [`DiscoveryOptions::TODAY`] (a thread
    /// that records nothing already means today's behaviour).
    pub record: Option<DiscoveryOptions>,
}

/// Decides a session turn's options.
///
/// `generation_start` is true for the turn that prepares the thread's first
/// prompt: it takes the options `config` asks for and records them, unless
/// `rows` already record some (a compacted head whose prefix is prepared
/// again keeps its pin). Every other turn reads what `rows` recorded, or
/// [`DiscoveryOptions::TODAY`].
///
/// With `config.force_pin` a turn on an existing thread takes the options
/// `config` asks for instead, and records them when they differ from what the
/// thread runs with. A change back to [`DiscoveryOptions::TODAY`] is recorded
/// too: a thread with no record means today's behaviour only until it has one.
pub fn resolve(
    config: &ToolSearchConfig,
    rows: &[TranscriptMessage],
    generation_start: bool,
) -> ResolvedDiscovery {
    let recorded = DiscoveryOptions::recorded_in(rows);
    if generation_start && recorded.is_none() {
        let effective = DiscoveryOptions::from_config(config);
        return ResolvedDiscovery {
            effective,
            record: (effective != DiscoveryOptions::TODAY).then_some(effective),
        };
    }
    let current = recorded.unwrap_or(DiscoveryOptions::TODAY);
    if config.force_pin {
        let effective = DiscoveryOptions::from_config(config);
        return ResolvedDiscovery {
            effective,
            record: (effective != current).then_some(effective),
        };
    }
    ResolvedDiscovery {
        effective: current,
        record: None,
    }
}

#[cfg(test)]
#[path = "pin_tests.rs"]
mod tests;
