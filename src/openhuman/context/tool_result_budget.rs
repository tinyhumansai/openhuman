//! Stage 1: Tool-result budget.
//!
//! Apply a per-call byte cap to a raw tool result *before* it enters the
//! conversation history. This is the cheapest stage because it operates
//! on fresh bytes that have not yet been sent to the inference backend —
//! it does not mutate existing history and therefore does not break the
//! KV-cache prefix.
//!
//! A future iteration could park the overflow in a "stored surrogate"
//! and reference it later if the model asks for the full body. For now
//! OpenHuman does the simpler thing: truncate in-place with a size
//! marker the model can use to decide whether to re-run the tool with a
//! narrower query.
//!
//! This stage is called from `Agent::execute_tool_call` once the tool
//! has returned its output and before that output is packaged into a
//! `ToolResultMessage`.

use std::fmt::Write as _;

/// Default per-tool-result budget. Large raw tool payloads are trimmed
/// inline before they enter history so parent-session tool output
/// cannot grow without bound. This remains compatible with the payload
/// summarizer: when summarization is enabled it can still replace very
/// large payloads earlier in the pipeline, and when it is disabled
/// (`summarizer_payload_threshold_tokens = 0`) this budget is the
/// default safeguard.
pub const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Number of trailing bytes reserved for the truncation marker. The
/// effective head capacity is `budget - TRAILER_RESERVED`.
const TRAILER_RESERVED: usize = 256;

/// Outcome of a budget application, for tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetOutcome {
    /// Byte length of the original content.
    pub original_bytes: usize,
    /// Byte length of the returned content (`== original_bytes` when the
    /// result fit inside the budget).
    pub final_bytes: usize,
    /// `true` if the content was truncated.
    pub truncated: bool,
}

impl BudgetOutcome {
    pub fn unchanged(len: usize) -> Self {
        Self {
            original_bytes: len,
            final_bytes: len,
            truncated: false,
        }
    }
}

/// Apply the tool-result budget to `content`.
///
/// If `content` fits in `budget_bytes`, returns it unchanged. Otherwise
/// returns a truncated prefix followed by a human-readable marker like
/// `\n\n[… 42_384 bytes truncated by tool_result_budget …]`. The cut is
/// made at a UTF-8 character boundary so the returned string is always
/// valid UTF-8.
pub fn apply_tool_result_budget(content: String, budget_bytes: usize) -> (String, BudgetOutcome) {
    let original_bytes = content.len();
    if budget_bytes == 0 || original_bytes <= budget_bytes {
        return (content, BudgetOutcome::unchanged(original_bytes));
    }

    // Reserve room for the trailer. If the budget is smaller than the
    // reservation we still emit the marker; the only guarantee is that
    // the final string is shorter than the original.
    let head_capacity = budget_bytes.saturating_sub(TRAILER_RESERVED).max(1);

    // Walk char indices forward until we cross the head capacity. The
    // last char fully inside the head is where we cut.
    let mut cut = crate::openhuman::util::floor_char_boundary(&content, head_capacity);

    // Extremely short content (single multi-byte char) — guarantee at
    // least one character makes it into the head so we don't emit a
    // zero-byte head.
    if cut == 0 {
        cut = content
            .char_indices()
            .next()
            .map(|(_, c)| c.len_utf8())
            .unwrap_or(0);
    }

    let dropped_bytes = original_bytes.saturating_sub(cut);
    let mut out = String::with_capacity(cut + TRAILER_RESERVED);
    out.push_str(&content[..cut]);
    // Hard separator so the marker is easy for humans AND the model to
    // recognise when it appears inside a tool_result block.
    let _ = write!(
        out,
        "\n\n[… {dropped_bytes} bytes truncated by tool_result_budget — re-run with a narrower query to see the rest …]"
    );

    let final_bytes = out.len();
    (
        out,
        BudgetOutcome {
            original_bytes,
            final_bytes,
            truncated: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_content_passes_through_unchanged() {
        let input = "hello world".to_string();
        let (out, outcome) = apply_tool_result_budget(input.clone(), 1024);
        assert_eq!(out, input);
        assert!(!outcome.truncated);
        assert_eq!(outcome.original_bytes, outcome.final_bytes);
    }

    #[test]
    fn content_at_exact_budget_is_unchanged() {
        let input = "x".repeat(100);
        let (out, outcome) = apply_tool_result_budget(input.clone(), 100);
        assert_eq!(out, input);
        assert!(!outcome.truncated);
    }

    #[test]
    fn oversized_content_is_truncated_with_marker() {
        let input = "x".repeat(10_000);
        let (out, outcome) = apply_tool_result_budget(input, 1024);
        assert!(outcome.truncated);
        assert!(out.len() < 10_000);
        assert!(out.contains("truncated by tool_result_budget"));
        // Marker should include the dropped byte count.
        assert!(out.contains("bytes truncated"));
    }

    #[test]
    fn truncation_respects_utf8_boundaries() {
        // Each "é" is 2 bytes. 600 of them = 1200 bytes.
        let input: String = "é".repeat(600);
        let (out, outcome) = apply_tool_result_budget(input, 500);
        assert!(outcome.truncated);
        // Must be valid UTF-8 — just dereferencing is enough.
        let _ = out.as_str();
        // Head should contain only full "é" characters (no half-byte).
        let head_end = out.find("\n\n[").unwrap();
        let head = &out[..head_end];
        assert!(head.chars().all(|c| c == 'é'));
    }

    #[test]
    fn zero_budget_is_noop() {
        let input = "keep me".to_string();
        let (out, outcome) = apply_tool_result_budget(input.clone(), 0);
        assert_eq!(out, input);
        assert!(!outcome.truncated);
    }

    #[test]
    fn outcome_reports_correct_byte_counts() {
        let input = "x".repeat(5_000);
        let (out, outcome) = apply_tool_result_budget(input, 1024);
        assert_eq!(outcome.original_bytes, 5_000);
        assert_eq!(outcome.final_bytes, out.len());
        assert!(outcome.truncated);
    }
}
