use super::*;
use crate::openhuman::inference::local::profile::LocalProviderKind;

#[test]
fn local_fallback_uses_profile_default() {
    // Unknown model with Ollama profile → 8192 default
    assert_eq!(
        context_window_for_model_with_local_fallback("qwen3:14b", Some(LocalProviderKind::Ollama)),
        Some(8_192)
    );
    // Unknown model with MLX profile → 4096 default
    assert_eq!(
        context_window_for_model_with_local_fallback(
            "my-custom-model",
            Some(LocalProviderKind::Mlx)
        ),
        Some(4_096)
    );
    // Unknown model with no local provider → None
    assert_eq!(
        context_window_for_model_with_local_fallback("qwen3:14b", None),
        None
    );
    // Local provider whose profile declares no default (llama.cpp / vLLM via
    // LocalOpenai) → conservative floor, NOT None. None here would disable
    // pre-dispatch trimming and let the prompt overflow the runtime n_ctx
    // (the TAURI-RUST-6V0 400). Must stay bounded.
    assert_eq!(
        context_window_for_model_with_local_fallback(
            "some-unlisted-gguf",
            Some(LocalProviderKind::LocalOpenai)
        ),
        Some(super::CONSERVATIVE_LOCAL_CONTEXT_FLOOR)
    );
    // Known model ignores local fallback
    assert_eq!(
        context_window_for_model_with_local_fallback("llama3:8b", Some(LocalProviderKind::Ollama)),
        Some(128_000)
    );
}

#[test]
fn tier_aliases_resolve() {
    assert_eq!(context_window_for_model("reasoning-v1"), Some(1_000_000));
    assert_eq!(context_window_for_model("agentic-v1"), Some(200_000));
    // chat-v1 is backed by DeepSeek v4 Flash (~1M), the same model as
    // summarization-v1 — not a 128K model (issue #4706).
    assert_eq!(context_window_for_model("chat-v1"), Some(1_000_000));
    // Burst tier — 128k on the managed backend. Matched on the alias, not
    // the local-gemma 8k substring arm.
    assert_eq!(context_window_for_model("burst-v1"), Some(128_000));
    // reasoning-quick-v1 is the legacy alias of chat-v1 (backend renamed it
    // 2026-05), so it resolves to the same ~1M flash window.
    assert_eq!(
        context_window_for_model("reasoning-quick-v1"),
        Some(1_000_000)
    );
    // summarization-v1 maps to a ~1M-token flash model so the extractor can
    // single-shot whole oversized payloads.
    assert_eq!(
        context_window_for_model("summarization-v1"),
        Some(1_000_000)
    );
    // The three flash-backed tiers share one window and must not drift.
    assert_eq!(
        context_window_for_model("chat-v1"),
        context_window_for_model("summarization-v1")
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
fn model_vision_enabled_reads_registry_only() {
    use crate::openhuman::config::schema::ModelRegistryEntry;
    let mut config = crate::openhuman::config::Config::default();
    config.model_registry = vec![
        ModelRegistryEntry {
            id: "my-llava".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: true,
            ..Default::default()
        },
        ModelRegistryEntry {
            id: "text-only".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: false,
            ..Default::default()
        },
    ];
    assert!(model_vision_enabled("my-llava", &config));
    assert!(!model_vision_enabled("text-only", &config));
    assert!(!model_vision_enabled("unlisted", &config));
    assert!(!model_vision_enabled("   ", &config));
}

#[test]
fn model_supports_vision_combines_tier_map_and_registry() {
    use crate::openhuman::config::schema::ModelRegistryEntry;
    let mut config = crate::openhuman::config::Config::default();
    config.model_registry = vec![ModelRegistryEntry {
        id: "my-llava".into(),
        provider: "openai".into(),
        cost_per_1m_output: 0.0,
        vision: true,
        ..Default::default()
    }];
    // `reasoning-v1` and `vision-v1` are the vision-capable managed tiers; the
    // rest are not.
    assert!(model_supports_vision("reasoning-v1", &config));
    assert!(model_supports_vision("hint:reasoning", &config));
    assert!(model_supports_vision("vision-v1", &config));
    assert!(model_supports_vision("hint:vision", &config));
    assert!(!model_supports_vision("chat-v1", &config));
    assert!(!model_supports_vision("hint:chat", &config));
    assert!(!model_supports_vision("burst-v1", &config));
    assert!(!model_supports_vision("hint:burst", &config));
    // BYOK model flagged in the registry is vision-capable.
    assert!(model_supports_vision("my-llava", &config));
    // Unlisted custom model is not.
    assert!(!model_supports_vision("gpt-5", &config));
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

/// Pins the complete managed-tier vision map — every tier constant and every
/// `hint:` alias `oh_tier_supports_vision` accepts.
///
/// The map's capability set is documented in three places (the
/// `inference.resolve_model` handler comment, the `oh_tier_supports_vision`
/// doc, and the [`model_supports_vision`] doc) and those comments drifted out
/// of sync with it once already (#6075: they claimed the map was "currently all
/// `false`" long after two tiers returned `true`). Asserting every arm — the
/// `false` ones included — means a future arm flip fails here and forces the
/// docs to be revisited, instead of only the two `true` arms being covered.
#[test]
fn oh_tier_vision_map_is_exhaustively_pinned() {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    use crate::openhuman::inference::provider::factory::oh_tier_supports_vision;

    for tier in [
        MODEL_REASONING_V1,
        "hint:reasoning",
        MODEL_VISION_V1,
        "hint:vision",
    ] {
        assert!(
            oh_tier_supports_vision(tier),
            "{tier} must be reported vision-capable"
        );
    }

    for tier in [
        MODEL_CHAT_V1,
        "hint:chat",
        MODEL_REASONING_QUICK_V1,
        MODEL_AGENTIC_V1,
        "hint:agentic",
        MODEL_BURST_V1,
        "hint:burst",
        MODEL_CODING_V1,
        "hint:coding",
        MODEL_SUMMARIZATION_V1,
        "hint:summarization",
        // Anything the map does not name at all falls through to `false`.
        "gpt-5",
        "",
    ] {
        assert!(
            !oh_tier_supports_vision(tier),
            "{tier} must not be reported vision-capable"
        );
    }
}
