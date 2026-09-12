use super::*;

// The chunk budget tracks the resolved context window, so a small local
// window yields a much smaller budget than a long-context cloud tier — this
// is what forces chunking instead of an oversized single-shot prompt.
#[test]
fn chunk_budget_tracks_context_window() {
    let summarization_window =
        crate::openhuman::inference::context_window_for_model("summarization-v1");
    let big = chunk_char_budget_for_window(summarization_window);
    let small = chunk_char_budget_for_window(Some(8_192)); // Ollama local default
    assert!(
        big > small,
        "long-context tier budget {big} must exceed an 8k local window budget {small}"
    );
}

// Codex P2: an unknown LOCAL model resolves (via the provider) to its small
// ~8k profile window, NOT the 128k cloud fallback. The resulting budget must
// be well under a production handoff payload (~200k chars) so it chunks
// instead of single-shotting into a local context overflow.
#[test]
fn chunk_budget_for_small_local_window_forces_chunking() {
    let budget = chunk_char_budget_for_window(Some(8_192));
    // 8192 * 70% * 4 = 22_937 chars.
    assert_eq!(budget, (8_192u64 * 70 / 100 * 4) as usize);
    assert!(
        budget < 200_000,
        "an 8k local window must budget below a typical handoff payload so it chunks"
    );
}

// When neither provider nor registry can size the model (cloud-unknown), the
// cloud-safe 128k fallback applies.
#[test]
fn chunk_budget_uses_cloud_fallback_when_unsizable() {
    let expected = (128_000u64 * 70 / 100 * 4) as usize;
    assert_eq!(chunk_char_budget_for_window(None), expected);
}
