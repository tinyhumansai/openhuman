use super::*;

#[test]
fn dedicated_vision_families_are_capable() {
    assert!(is_vision_capable("moondream:1.8b-v2-q4_K_S"));
    assert!(is_vision_capable("moondream"));
    assert!(is_vision_capable("llava:7b"));
    assert!(is_vision_capable("llava:13b"));
    assert!(is_vision_capable("bakllava:latest"));
    assert!(is_vision_capable("llama3.2-vision:11b"));
    assert!(is_vision_capable("minicpm-v:8b"));
    assert!(is_vision_capable("qwen2.5vl:7b"));
}

#[test]
fn gemma3_is_multimodal_only_at_4b_and_above() {
    // 270m / 1b ship without a vision encoder.
    assert!(!is_vision_capable("gemma3:270m-it-qat"));
    assert!(!is_vision_capable("gemma3:1b-it-qat"));
    assert!(!is_vision_capable("gemma3:1b"));

    assert!(is_vision_capable("gemma3:4b-it-qat"));
    assert!(is_vision_capable("gemma3:12b-it-qat"));
    assert!(is_vision_capable("gemma3:27b"));
    // `latest` is the 4B multimodal build.
    assert!(is_vision_capable("gemma3:latest"));
    assert!(is_vision_capable("gemma3"));
}

#[test]
fn gemma3n_is_never_treated_as_vision_capable() {
    // Regression guard for #5146: `gemma3n` shares a prefix with `gemma3`
    // but is text-only, and it was previously wired in as the 16 GB+
    // tier's vision model.
    assert!(!is_vision_capable("gemma3n:e4b-it-q8_0"));
    assert!(!is_vision_capable("gemma3n:e2b"));
    assert!(!is_vision_capable("gemma3n"));
    assert!(!is_vision_capable("GEMMA3N:E4B-IT-Q8_0"));
}

#[test]
fn gemma4_is_multimodal_at_every_size() {
    assert!(is_vision_capable("gemma4:e4b-it-q8_0"));
    assert!(is_vision_capable("gemma4:e2b-it-qat"));
    assert!(is_vision_capable("gemma4:12b"));
    assert!(is_vision_capable("gemma4"));
}

#[test]
fn chat_only_models_are_rejected() {
    assert!(!is_vision_capable("llama3.1:8b"));
    assert!(!is_vision_capable("qwen2.5:14b"));
    assert!(!is_vision_capable("deepseek-r1:7b"));
    assert!(!is_vision_capable("phi4:latest"));
}

#[test]
fn embedding_models_are_rejected() {
    assert!(!is_vision_capable("bge-m3"));
    assert!(!is_vision_capable("all-minilm:latest"));
    assert!(!is_vision_capable("nomic-embed-text:latest"));
}

#[test]
fn empty_and_whitespace_are_rejected() {
    assert!(!is_vision_capable(""));
    assert!(!is_vision_capable("   "));
}

#[test]
fn repackaged_upstream_vision_models_are_detected() {
    assert!(is_vision_capable("hf.co/user/llava-v1.6-mistral-7b"));
    assert!(is_vision_capable("my-moondream:custom"));
    assert!(is_vision_capable("someone/llama3.2-vision-abliterated"));
}

#[test]
fn detection_is_case_insensitive() {
    assert!(is_vision_capable("LLaVA:7B"));
    assert!(is_vision_capable("MoonDream"));
    assert!(!is_vision_capable("Gemma3:1B-IT-QAT"));
}

#[test]
fn every_suggested_vision_model_is_vision_capable() {
    // The suggestions are quoted verbatim in the user-facing errors from
    // `model_ids::resolve_vision_model_id` — both the "nothing configured"
    // and the "not vision-capable" arms. A chat-only entry here would send
    // a user straight back into the error they are trying to escape.
    for suggestion in VISION_MODEL_SUGGESTIONS {
        assert!(
            is_vision_capable(suggestion),
            "suggested vision model `{suggestion}` is not vision-capable"
        );
    }
}
