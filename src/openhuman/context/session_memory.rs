//! Stage 5: Session memory — persistent notes updated by a background fork.
//!
//! Session memory is intentionally **separate** from compaction. While
//! microcompact/autocompact mutate the in-flight conversation history to
//! keep the prompt inside the context window, session memory is a
//! persistent markdown file (`MEMORY.md` in the workspace) that survives
//! across sessions and acts as the long-term substrate the next session
//! hydrates from. It is updated by a background forked sub-agent (the
//! `archivist` archetype) so the user-facing agent never pays the cost
//! of synthesis on its hot path.
//!
//! Extraction only runs after token-growth, tool-call, and turn-count
//! thresholds are met, so it does not fire every turn — see
//! [`SessionMemoryConfig`] for the exact knobs.
//!
//! This module is purely state-tracking: it owns the thresholds and a
//! `should_extract` decision, but the actual `spawn_subagent` call is
//! issued by the caller (the `Agent::turn` epilogue) so we avoid a
//! circular dependency with `harness::subagent_runner`.

/// Minimum number of *new* tokens (input + output) since the last
/// extraction before we consider running another extraction.
pub const DEFAULT_MIN_TOKEN_GROWTH: u64 = 4_000;

/// Minimum number of assistant tool calls since the last extraction
/// before we consider running another extraction.
pub const DEFAULT_MIN_TOOL_CALLS: u64 = 8;

/// Minimum number of turns between extractions. Prevents burst
/// extraction when the user sends many short messages in a row.
pub const DEFAULT_MIN_TURNS_BETWEEN: u64 = 4;

/// Tunable thresholds for session-memory extraction.
///
/// Serializable so it can be embedded directly into the top-level
/// [`crate::openhuman::config::ContextConfig`] config section.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionMemoryConfig {
    #[serde(default = "default_min_token_growth")]
    pub min_token_growth: u64,
    #[serde(default = "default_min_tool_calls")]
    pub min_tool_calls: u64,
    #[serde(default = "default_min_turns_between")]
    pub min_turns_between: u64,
}

fn default_min_token_growth() -> u64 {
    DEFAULT_MIN_TOKEN_GROWTH
}

fn default_min_tool_calls() -> u64 {
    DEFAULT_MIN_TOOL_CALLS
}

fn default_min_turns_between() -> u64 {
    DEFAULT_MIN_TURNS_BETWEEN
}

impl Default for SessionMemoryConfig {
    fn default() -> Self {
        Self {
            min_token_growth: DEFAULT_MIN_TOKEN_GROWTH,
            min_tool_calls: DEFAULT_MIN_TOOL_CALLS,
            min_turns_between: DEFAULT_MIN_TURNS_BETWEEN,
        }
    }
}

/// Per-session extraction state. Tracked on the `Agent` instance so it
/// resets naturally when a new session starts.
#[derive(Debug, Clone, Default)]
pub struct SessionMemoryState {
    /// Cumulative tokens observed across the whole session (via
    /// `ContextGuard::update_usage`).
    pub total_tokens: u64,
    /// Tokens at the last completed extraction (or 0 if none yet).
    pub tokens_at_last_extract: u64,
    /// Turn counter at the last completed extraction.
    pub turn_at_last_extract: u64,
    /// Cumulative tool-call count across the session.
    pub total_tool_calls: u64,
    /// Tool calls observed at the last extraction.
    pub tool_calls_at_last_extract: u64,
    /// Current turn counter.
    pub current_turn: u64,
    /// Whether an extraction is in progress. While `true`, `should_extract`
    /// returns false so we don't spawn overlapping background forks.
    pub extraction_in_progress: bool,
}

impl SessionMemoryState {
    /// Called each time the caller bumps the turn counter.
    pub fn tick_turn(&mut self) {
        self.current_turn = self.current_turn.saturating_add(1);
    }

    /// Accumulate usage from the most recent provider response.
    pub fn record_usage(&mut self, total_used_tokens: u64) {
        // `total_used_tokens` is cumulative per-response (prompt + output);
        // we want monotonic growth so take the max against what we've
        // already recorded. This is robust to providers that report
        // smaller numbers when tool-only turns happen.
        if total_used_tokens > self.total_tokens {
            self.total_tokens = total_used_tokens;
        }
    }

    /// Accumulate a tool-call count from the turn just finished.
    pub fn record_tool_calls(&mut self, n: usize) {
        self.total_tool_calls = self.total_tool_calls.saturating_add(n as u64);
    }

    /// Decide whether a background session-memory extraction should run
    /// right now. The rule: all three deltas (tokens, tool calls, turns)
    /// must have grown past their thresholds since the last extraction,
    /// AND no other extraction is in flight.
    pub fn should_extract(&self, config: &SessionMemoryConfig) -> bool {
        if self.extraction_in_progress {
            return false;
        }
        let token_growth = self
            .total_tokens
            .saturating_sub(self.tokens_at_last_extract);
        let tool_growth = self
            .total_tool_calls
            .saturating_sub(self.tool_calls_at_last_extract);
        let turn_growth = self.current_turn.saturating_sub(self.turn_at_last_extract);

        token_growth >= config.min_token_growth
            && tool_growth >= config.min_tool_calls
            && turn_growth >= config.min_turns_between
    }

    /// Mark an extraction as in-progress. Must be paired with either
    /// `mark_extraction_complete` or `mark_extraction_failed`.
    pub fn mark_extraction_started(&mut self) {
        self.extraction_in_progress = true;
    }

    /// Record a successful extraction. Resets the deltas so the next
    /// extraction won't fire until the thresholds are re-crossed.
    pub fn mark_extraction_complete(&mut self) {
        self.extraction_in_progress = false;
        self.tokens_at_last_extract = self.total_tokens;
        self.tool_calls_at_last_extract = self.total_tool_calls;
        self.turn_at_last_extract = self.current_turn;
    }

    /// Record a failed extraction. Leaves the deltas alone so the next
    /// turn can retry, but clears the in-progress flag.
    pub fn mark_extraction_failed(&mut self) {
        self.extraction_in_progress = false;
    }
}

/// The prompt the main agent hands to a spawned archivist sub-agent when
/// session-memory extraction fires. Kept in this module so the
/// extraction policy and the spawn wording live together.
pub const ARCHIVIST_EXTRACTION_PROMPT: &str =
    "You are extracting durable facts from the recent conversation \
into the workspace `MEMORY.md` file. Focus on:\n\n\
- User preferences and commitments\n\
- Decisions and their rationale\n\
- Facts about external systems, people, codebases the user mentioned\n\
- Unresolved tasks worth surfacing next session\n\n\
Skip: filler dialogue, tool logs, and anything already present in \
MEMORY.md. Use the `update_memory_md` tool to append a dated bullet \
list under an `## Observations` section. Be dense — at most 8 bullets. \
Reply with a one-line confirmation when done.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_does_not_extract() {
        let state = SessionMemoryState::default();
        let cfg = SessionMemoryConfig::default();
        assert!(!state.should_extract(&cfg));
    }

    #[test]
    fn all_three_thresholds_must_be_crossed() {
        let cfg = SessionMemoryConfig::default();

        // Only token threshold crossed → no.
        let mut s = SessionMemoryState::default();
        s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
        assert!(!s.should_extract(&cfg));

        // Tokens + tool calls, no turn growth → no.
        s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
        assert!(!s.should_extract(&cfg));

        // All three crossed → yes.
        s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
        assert!(s.should_extract(&cfg));
    }

    #[test]
    fn in_progress_suppresses_extraction() {
        let cfg = SessionMemoryConfig::default();
        let mut s = SessionMemoryState::default();
        s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
        s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
        s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
        assert!(s.should_extract(&cfg));
        s.mark_extraction_started();
        assert!(!s.should_extract(&cfg));
    }

    #[test]
    fn mark_complete_resets_deltas() {
        let cfg = SessionMemoryConfig::default();
        let mut s = SessionMemoryState::default();
        s.total_tokens = 10_000;
        s.total_tool_calls = 15;
        s.current_turn = 10;
        s.mark_extraction_started();
        s.mark_extraction_complete();

        // Immediately after completion no further extraction should
        // fire until the deltas are re-crossed.
        assert!(!s.should_extract(&cfg));

        // Grow each counter past threshold again.
        s.total_tokens += DEFAULT_MIN_TOKEN_GROWTH;
        s.total_tool_calls += DEFAULT_MIN_TOOL_CALLS;
        s.current_turn += DEFAULT_MIN_TURNS_BETWEEN;
        assert!(s.should_extract(&cfg));
    }

    #[test]
    fn mark_failed_leaves_deltas_intact() {
        let cfg = SessionMemoryConfig::default();
        let mut s = SessionMemoryState::default();
        s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
        s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
        s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
        s.mark_extraction_started();
        s.mark_extraction_failed();

        // Should still fire on the next attempt because the
        // "last_extract" counters were not advanced.
        assert!(s.should_extract(&cfg));
    }

    #[test]
    fn record_usage_is_monotonic() {
        let mut s = SessionMemoryState::default();
        s.record_usage(5_000);
        s.record_usage(3_000); // regression — must not decrease.
        assert_eq!(s.total_tokens, 5_000);
        s.record_usage(7_500);
        assert_eq!(s.total_tokens, 7_500);
    }

    #[test]
    fn tick_turn_increments() {
        let mut s = SessionMemoryState::default();
        s.tick_turn();
        s.tick_turn();
        s.tick_turn();
        assert_eq!(s.current_turn, 3);
    }

    #[test]
    fn record_tool_calls_accumulates() {
        let mut s = SessionMemoryState::default();
        s.record_tool_calls(3);
        s.record_tool_calls(2);
        assert_eq!(s.total_tool_calls, 5);
    }
}
