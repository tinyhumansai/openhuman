use super::*;

/// The defect this text exists to prevent: shown a sample handle, two live
/// runs called the tool with `res_1` against a cache that had never issued
/// one. Nothing model-facing here may contain a handle-shaped literal — a
/// description is a prompt, and an example in it is an instruction.
#[test]
fn no_model_facing_text_shows_a_fabricated_handle() {
    let corpus = format!("{} {}", description_text(), parameters_schema_json());

    for fabricated in ["res_1", "res_0", "res_123", "\"res_", "result_1"] {
        assert!(
            !corpus.contains(fabricated),
            "model-facing text must not show `{fabricated}` — the model sends what it is shown"
        );
    }
}

/// The other half of the same contract: having removed the example, the
/// text must still say where a real handle comes from, and that it can stop
/// working. Without the second part an evicted handle reads to the model as
/// "wrong handle" and invites a guess, which is where the fabrication came
/// from in the first place.
#[test]
fn the_handle_argument_says_where_a_handle_comes_from_and_that_it_expires() {
    let schema = parameters_schema_json();
    let desc = schema["properties"]["result_id"]["description"]
        .as_str()
        .expect("result_id documents itself");

    assert!(
        desc.contains("appeared in an earlier result"),
        "the only valid source of a handle must be stated: {desc}"
    );
    for expiry_term in ["evicted", "re-run the original tool"] {
        assert!(
            desc.to_lowercase().contains(&expiry_term.to_lowercase()),
            "handle expiry must be documented (`{expiry_term}` missing): {desc}"
        );
    }
}

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
