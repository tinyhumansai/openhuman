use super::*;

fn test_config() -> Config {
    Config::default()
}

#[test]
fn chat_model_falls_back_for_empty_and_unsupported_ids() {
    let mut config = test_config();

    config.local_ai.chat_model_id = String::new();
    config.local_ai.model_id = String::new();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

    config.local_ai.chat_model_id = "custom.gguf".to_string();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

    config.local_ai.chat_model_id = "qwen3-1.7b".to_string();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
}

#[test]
fn chat_model_allows_mvp_model() {
    let mut config = test_config();
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    assert_eq!(effective_chat_model_id(&config), "gemma3:1b-it-qat");
}

#[test]
fn chat_model_allows_requested_ollama_gemma3n_q8() {
    let mut config = test_config();
    config.local_ai.chat_model_id = "gemma3n:e4b-it-q8_0".to_string();
    assert_eq!(effective_chat_model_id(&config), "gemma3n:e4b-it-q8_0");
}

#[test]
fn chat_model_allows_custom_ids_for_lm_studio() {
    let mut config = test_config();
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.chat_model_id = "publisher/custom-model-7b".to_string();
    assert_eq!(
        effective_chat_model_id(&config),
        "publisher/custom-model-7b"
    );
}

#[test]
fn lm_studio_chat_model_returns_empty_when_no_model_configured() {
    // LM Studio has no sensible Ollama-branded default — an empty model ID
    // surfaces the missing-model warning in diagnostics / status rather than
    // silently sending "gemma3:1b-it-qat" to an LM Studio server.
    let mut config = test_config();
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.chat_model_id = String::new();
    config.local_ai.model_id = String::new();
    assert_eq!(effective_chat_model_id(&config), "");
}

#[test]
fn chat_model_rejects_non_mvp_models() {
    let mut config = test_config();

    // Bare `gemma3n:e4b` is a real Ollama tag but is NOT the allowlisted
    // quantization, so it still redirects to the default.
    config.local_ai.chat_model_id = "gemma3n:e4b".to_string();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

    // Arbitrary non-preset models stay rejected.
    config.local_ai.chat_model_id = "llama3.1:8b".to_string();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

    config.local_ai.chat_model_id = "totally-made-up-model:v0".to_string();
    assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
}

/// #5146 §1.3: the allowlist must cover every preset chat model.
///
/// `gemma3:270m-it-qat` (1 GB tier) and `gemma3:4b-it-qat` (8-16 GB tier)
/// were previously absent, so applying either preset resolved straight
/// back to the 1B default — the user picked a tier and silently got a
/// different model than the one the preset advertised.
#[test]
fn preset_chat_models_are_allowlisted_and_resolve_unchanged() {
    let mut config = test_config();
    for preset in crate::openhuman::inference::presets::all_presets() {
        config.local_ai.chat_model_id = preset.chat_model_id.to_string();
        assert_eq!(
            effective_chat_model_id(&config),
            preset.chat_model_id,
            "preset {:?} chat model `{}` is not allowlisted and was redirected",
            preset.tier,
            preset.chat_model_id
        );
    }
}

/// GH #5055 / #5146 §1.3: every allowlisted chat model must be a real,
/// fully-qualified Ollama id.
///
/// The #5055 form of this test asserted "no entry may start with
/// `gemma4:`", because no `gemma4` namespace existed at the time. Gemma 4
/// has since been published and `gemma4:e4b-it-q8_0` resolves against
/// `registry.ollama.ai`, so that assertion was pinning an expired fact.
/// The durable invariant is the `<model>:<tag>` shape plus the
/// preset cross-check above.
#[test]
fn mvp_chat_allowlist_entries_are_fully_qualified() {
    for model in MVP_ALLOWED_CHAT_MODELS {
        assert!(
            model.contains(':'),
            "`{model}` must be a fully-qualified `<model>:<tag>` id"
        );
    }
}

#[test]
fn vision_model_normalizes_legacy_moondream_values() {
    let mut config = test_config();

    // Empty stays empty: "vision not configured" is a real state.
    config.local_ai.vision_model_id = String::new();
    assert_eq!(effective_vision_model_id(&config), "");

    // Legacy shorthands normalize to the pinned Moondream build. Before
    // #5146 these resolved to "" (vision silently disabled) because the
    // vision allowlist contained only the empty string.
    config.local_ai.vision_model_id = "moondream".to_string();
    assert_eq!(effective_vision_model_id(&config), DEFAULT_LOW_VISION_MODEL);
    config.local_ai.vision_model_id = "moondream:1.8b".to_string();
    assert_eq!(effective_vision_model_id(&config), DEFAULT_LOW_VISION_MODEL);
}

/// #5146 §Part 1: a genuinely vision-capable model must survive resolution
/// unchanged. The previous `MVP_ALLOWED_VISION_MODELS = &[""]` allowlist
/// rewrote every one of these to `""`.
#[test]
fn vision_capable_models_pass_through_unchanged() {
    let mut config = test_config();
    for model in ["llava:7b", "gemma3:4b-it-qat", "gemma4:e4b-it-q8_0"] {
        config.local_ai.vision_model_id = model.to_string();
        assert_eq!(effective_vision_model_id(&config), model);
    }
}

/// #5146 §Part 1 / P1: a chat-only model must never be returned as the
/// vision model — and must not be quietly swapped for one either. Both
/// resolvers report it as unusable so no pull path can act on it.
#[test]
fn chat_only_vision_model_resolves_to_nothing_usable() {
    let mut config = test_config();
    for chat_only in ["gemma3n:e4b-it-q8_0", "gemma3:1b-it-qat", "llama3.1:8b"] {
        config.local_ai.vision_model_id = chat_only.to_string();
        assert_eq!(
            effective_vision_model_id(&config),
            "",
            "{chat_only} must not resolve to a substitute"
        );
        assert!(
            resolve_vision_model_id(&config).is_err(),
            "{chat_only} must be an actionable error at request time"
        );
    }
}

/// The pinned default must itself be vision-capable: it is what the
/// `moondream` alias resolves to, and what the "for example …" suggestions
/// point users at, so a chat-only default would send them in a circle.
#[test]
fn default_vision_model_is_vision_capable() {
    assert!(!DEFAULT_LOW_VISION_MODEL.is_empty());
    assert!(vision_models::is_vision_capable(DEFAULT_LOW_VISION_MODEL));
}

/// #5146 §Part 1: an unconfigured vision model must produce an actionable
/// error, not an empty model id that downstream code sends to Ollama.
#[test]
fn resolve_vision_model_id_errors_when_unconfigured() {
    let mut config = test_config();
    config.local_ai.vision_model_id = String::new();

    let err = resolve_vision_model_id(&config)
        .err()
        .expect("expected a vision error");
    assert!(
        err.contains("vision_model_id"),
        "error should name the config key to set: {err}"
    );
    assert!(
        err.contains("ollama pull"),
        "error should say how to install a model: {err}"
    );
    // Whitespace-only is the same "not configured" state.
    config.local_ai.vision_model_id = "   ".to_string();
    assert!(resolve_vision_model_id(&config).is_err());
}

#[test]
fn resolve_vision_model_id_returns_the_configured_model_when_it_can_see() {
    let mut config = test_config();
    config.local_ai.vision_model_id = "llava:7b".to_string();
    assert_eq!(resolve_vision_model_id(&config).unwrap(), "llava:7b");
}

/// An alias rewrite resolves to a different string but is the *same* model
/// the user asked for, so it stays silent and must keep working.
#[test]
fn resolve_vision_model_id_still_applies_the_moondream_alias() {
    let mut config = test_config();
    for alias in ["moondream", "moondream:1.8b", "MoonDream"] {
        config.local_ai.vision_model_id = alias.to_string();
        let resolved = resolve_vision_model_id(&config)
            .unwrap_or_else(|e| panic!("alias {alias} must resolve, got: {e}"));
        assert_eq!(resolved, DEFAULT_LOW_VISION_MODEL);
        assert!(vision_models::is_vision_capable(&resolved));
    }
}

// ── #5146 P1: a chat-only vision model errors, never substitutes ─────────

/// The headline P1 regression. A chat-only `vision_model_id` used to be
/// silently swapped for the default vision model, which was then
/// auto-pulled (~1.7 GB, no progress) and answered many prompts with an
/// empty string — surfacing as `ollama vision returned empty content`.
#[test]
fn resolve_vision_model_id_errors_on_a_chat_only_model_instead_of_substituting() {
    let mut config = test_config();
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();

    let err = resolve_vision_model_id(&config)
        .err()
        .expect("a chat-only vision model must be an error, not a substitution");

    assert!(
        err.contains("gemma3n:e4b-it-q8_0"),
        "the error must name the model the user actually configured: {err}"
    );
    assert!(
        err.contains("not vision-capable"),
        "the error must say what is wrong with it: {err}"
    );
    assert!(
        err.contains("vision_model_id"),
        "the error must name the key to change: {err}"
    );
    // `DEFAULT_LOW_VISION_MODEL` is also `VISION_MODEL_SUGGESTIONS[0]`, so
    // asserting its absence would assert against the suggestion list
    // itself. The contract is the framing: it is offered as one example to
    // pick from, not announced as the model that replaced the user's.
    assert!(
        err.contains("for example"),
        "a vision-capable model must be offered as an example to choose, never as a \
         substitute that was already applied: {err}"
    );
    assert!(
        !err.contains("selected vision model `moondream"),
        "the error must name the user's model as the problem, not a suggestion: {err}"
    );
}

/// The auto-pull half of P1: several callers feed
/// `effective_vision_model_id` straight into `ensure_ollama_model_available`,
/// so a substituted id here is exactly how an unchosen model got downloaded.
/// Empty keeps those paths off it (and `ensure_ollama_model_available`
/// rejects a blank id rather than pulling a nameless model).
#[test]
fn effective_vision_model_id_is_empty_for_a_chat_only_model() {
    let mut config = test_config();
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    assert_eq!(
        effective_vision_model_id(&config),
        "",
        "a chat-only model must not resolve to a substitute that a pull path would download"
    );

    // Unchanged for the two states that already worked.
    config.local_ai.vision_model_id = String::new();
    assert_eq!(effective_vision_model_id(&config), "");
    config.local_ai.vision_model_id = "llava:7b".to_string();
    assert_eq!(effective_vision_model_id(&config), "llava:7b");
}

/// `effective_vision_model_id` and `resolve_vision_model_id` must agree on
/// which models are usable — a non-empty effective id that the resolver
/// rejects (or vice versa) would put status surfaces and request-time
/// behaviour out of sync.
#[test]
fn effective_and_resolved_vision_ids_agree_on_usability() {
    let mut config = test_config();
    for candidate in [
        "llava:7b",
        "gemma3n:e4b-it-q8_0",
        "moondream",
        "llama3.2:3b",
        "",
    ] {
        config.local_ai.vision_model_id = candidate.to_string();
        let effective = effective_vision_model_id(&config);
        let resolved = resolve_vision_model_id(&config);
        assert_eq!(
            effective.is_empty(),
            resolved.is_err(),
            "disagreement for {candidate:?}: effective={effective:?} resolved={resolved:?}"
        );
        if let Ok(model) = resolved {
            assert_eq!(effective, model);
        }
    }
}

#[test]
fn embedding_model_empty_falls_back_to_bge_m3() {
    // After the cloud-embeddings unification PR, the default embedder
    // for the local Ollama path is bge-m3 (1024 dim) to match memory
    // tree's fixed on-disk format. Empty / whitespace input must
    // resolve to that default, not the prior all-minilm:latest.
    let mut config = test_config();
    config.local_ai.embedding_model_id = String::new();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");

    config.local_ai.embedding_model_id = "   ".to_string();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");
}

#[test]
fn embedding_model_passes_through_allowlisted_legacy() {
    // all-minilm:latest is kept in MVP_ALLOWED_EMBEDDING_MODELS for
    // back-compat with users who already pulled it under the prior
    // default. It is NOT 1024-dim — memory tree's post-call validator
    // will surface that mismatch at embed time — but the allowlist
    // enforcer itself must let the value pass through unchanged.
    let mut config = test_config();
    config.local_ai.embedding_model_id = "all-minilm:latest".to_string();
    assert_eq!(effective_embedding_model_id(&config), "all-minilm:latest");
}

#[test]
fn embedding_model_rejects_non_allowlisted_and_redirects_to_default() {
    // Any non-allowlisted value (including legacy nomic-embed-text:latest
    // and arbitrary user input) is silently redirected to the canonical
    // default. This is the path that fired the "embedding model not in
    // MVP allowlist, redirecting to default" warning on every embed
    // resolution before bge-m3 was added to the allowlist.
    let mut config = test_config();
    config.local_ai.embedding_model_id = "nomic-embed-text:latest".to_string();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");

    config.local_ai.embedding_model_id = "totally-made-up-model:v0".to_string();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");
}

#[test]
fn lm_studio_embedding_model_passes_through_served_name() {
    // The native local-runtime fix for #3920: LM Studio serves embeddings
    // under user-managed names that are not in the MVP allowlist. A
    // configured id must reach the runtime unchanged rather than being
    // rewritten back to bge-m3 (which the LM Studio server would not have
    // under that exact name).
    let mut config = test_config();
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.embedding_model_id = "text-embedding-bge-m3".to_string();
    assert_eq!(
        effective_embedding_model_id(&config),
        "text-embedding-bge-m3"
    );
}

#[test]
fn lm_studio_embedding_model_passes_through_arbitrary_id() {
    // Contrast with `embedding_model_rejects_non_allowlisted_and_redirects_to_default`:
    // the SAME non-allowlisted id is rewritten to bge-m3 on the managed
    // Ollama path but passes through unchanged on the LM Studio path.
    let mut config = test_config();
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.embedding_model_id = "nomic-embed-text:latest".to_string();
    assert_eq!(
        effective_embedding_model_id(&config),
        "nomic-embed-text:latest"
    );
}

#[test]
fn lm_studio_embedding_model_empty_falls_back_to_default() {
    // With no configured embedding id, fall back to the canonical default
    // so the memory tree still has an embedder to request.
    let mut config = test_config();
    config.local_ai.provider = "lm_studio".to_string();
    config.local_ai.embedding_model_id = String::new();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");

    config.local_ai.embedding_model_id = "   ".to_string();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");
}

#[test]
fn ollama_embedding_path_still_enforces_allowlist_after_lm_studio_bypass() {
    // Guard: the LM Studio bypass must not weaken the managed Ollama path.
    // Default provider (Ollama) still rewrites a non-allowlisted id.
    let mut config = test_config();
    config.local_ai.embedding_model_id = "text-embedding-bge-m3".to_string();
    assert_eq!(effective_embedding_model_id(&config), "bge-m3");
}

#[test]
fn stt_tts_and_quantization_defaults_are_applied() {
    let mut config = test_config();
    config.local_ai.stt_model_id.clear();
    config.local_ai.tts_voice_id.clear();
    config.local_ai.quantization = "Q5_K_M".to_string();

    assert_eq!(effective_stt_model_id(&config), "ggml-base-q5_1.bin");
    assert_eq!(effective_tts_voice_id(&config), "en_US-lessac-medium");
    assert_eq!(effective_quantization(&config), "q5_k_m");
}
