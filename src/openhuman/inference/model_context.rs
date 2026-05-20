//! Known model context-window sizes for pre-inference budgeting.
//!
//! Provider `/models` responses may include `context_length` / `context_window`,
//! but the agent harness must enforce limits **before** the first dispatch —
//! otherwise long histories produce upstream `400 Bad Request` errors when usage
//! metadata is not yet available.

use crate::openhuman::config::{
    MODEL_AGENTIC_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1, MODEL_REASONING_V1,
};

/// Conservative default for OpenHuman abstract tier models (tokens).
const TIER_LARGE_CONTEXT: u64 = 200_000;
const TIER_STANDARD_CONTEXT: u64 = 128_000;
const TIER_LOCAL_CONTEXT: u64 = 8_192;

/// How a pattern in [`MODEL_CONTEXT_PATTERNS`] is matched against a model id.
#[derive(Copy, Clone)]
enum PatternMatch {
    /// Pattern must appear anywhere as a substring (after lowercasing).
    Substring,
    /// Pattern must appear as a full `-`/`_`/`/`/`:`-delimited segment.
    /// Prevents false positives like `"solo1-7b"` matching the `"o1"` pattern
    /// or `"proto3-chat"` matching the `"o3"` pattern.
    Segment,
}

/// `(pattern, match mode, context window in tokens)` — first match wins.
const MODEL_CONTEXT_PATTERNS: &[(&str, PatternMatch, u64)] = &[
    ("claude-haiku-4.5", PatternMatch::Substring, 200_000),
    ("claude-haiku-4", PatternMatch::Substring, 200_000),
    ("claude-haiku", PatternMatch::Substring, 200_000),
    ("claude-sonnet-4", PatternMatch::Substring, 200_000),
    ("claude-opus-4", PatternMatch::Substring, 200_000),
    ("claude-3-5-sonnet", PatternMatch::Substring, 200_000),
    ("claude-3-5-haiku", PatternMatch::Substring, 200_000),
    ("claude-3-opus", PatternMatch::Substring, 200_000),
    ("gpt-4.1", PatternMatch::Substring, 1_047_576),
    ("gpt-4o", PatternMatch::Substring, 128_000),
    ("gpt-4-turbo", PatternMatch::Substring, 128_000),
    ("gpt-4", PatternMatch::Substring, 128_000),
    ("gpt-3.5", PatternMatch::Substring, 16_385),
    // `o1`/`o3` are short and collide with substrings of unrelated model ids
    // (e.g. `solo1-7b`, `proto3-chat`). Require a segment-boundary match.
    ("o1", PatternMatch::Segment, 200_000),
    ("o3", PatternMatch::Segment, 200_000),
    ("deepseek", PatternMatch::Substring, 128_000),
    ("gemma3", PatternMatch::Substring, 8_192),
    ("gemma", PatternMatch::Substring, 8_192),
    ("llama-3", PatternMatch::Substring, 128_000),
    ("llama3", PatternMatch::Substring, 128_000),
];

fn matches_pattern(lower: &str, pattern: &str, mode: PatternMatch) -> bool {
    match mode {
        PatternMatch::Substring => lower.contains(pattern),
        PatternMatch::Segment => lower
            .split(|c: char| matches!(c, '/' | '-' | '_' | ':' | '.'))
            .any(|seg| seg == pattern),
    }
}

/// Resolve the context window (in tokens) for a model id or OpenHuman tier alias.
///
/// Returns `None` when the model is unknown — callers should skip pre-dispatch
/// trimming rather than guess.
pub fn context_window_for_model(model: &str) -> Option<u64> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(window) = tier_context_window(normalized) {
        return Some(window);
    }

    let lower = normalized.to_ascii_lowercase();
    for (pattern, mode, window) in MODEL_CONTEXT_PATTERNS {
        if matches_pattern(&lower, pattern, *mode) {
            tracing::debug!(
                model = normalized,
                pattern,
                context_window = window,
                "[model_context] matched known model pattern"
            );
            return Some(*window);
        }
    }

    None
}

fn tier_context_window(model: &str) -> Option<u64> {
    match model {
        MODEL_REASONING_V1 | MODEL_AGENTIC_V1 | MODEL_CODING_V1 => Some(TIER_LARGE_CONTEXT),
        MODEL_REASONING_QUICK_V1 | "summarization-v1" | "chat" => Some(TIER_STANDARD_CONTEXT),
        m if m.starts_with("gemma") || m.contains(":1b") || m.contains("270m") => {
            Some(TIER_LOCAL_CONTEXT)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_aliases_resolve() {
        assert_eq!(context_window_for_model("reasoning-v1"), Some(200_000));
        assert_eq!(context_window_for_model("agentic-v1"), Some(200_000));
        assert_eq!(
            context_window_for_model("reasoning-quick-v1"),
            Some(128_000)
        );
    }

    #[test]
    fn copilot_haiku_resolves_to_200k() {
        assert_eq!(
            context_window_for_model("github_copilot/claude-haiku-4.5"),
            Some(200_000)
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(context_window_for_model("totally-unknown-model-xyz"), None);
    }

    #[test]
    fn empty_model_returns_none() {
        assert_eq!(context_window_for_model("   "), None);
    }

    #[test]
    fn o1_o3_segment_match_does_not_overmatch() {
        // Real OpenAI o1/o3 model ids must still resolve.
        assert_eq!(context_window_for_model("o1"), Some(200_000));
        assert_eq!(context_window_for_model("o1-mini"), Some(200_000));
        assert_eq!(context_window_for_model("o3-mini"), Some(200_000));
        assert_eq!(context_window_for_model("openai/o1-preview"), Some(200_000));

        // Names that merely *contain* the substring "o1" / "o3" must NOT
        // inherit the 200K window (regression guard for PR #2100 review).
        assert_eq!(context_window_for_model("solo1-7b"), None);
        assert_eq!(context_window_for_model("proto3-chat"), None);
        assert_eq!(
            context_window_for_model("ollama/mistral-for-o1-benchmark"),
            Some(200_000),
            "`-o1-` segment should still match"
        );
        assert_eq!(context_window_for_model("octo3thing"), None);
    }
}
