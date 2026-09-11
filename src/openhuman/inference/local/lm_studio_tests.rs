use super::*;

#[test]
fn native_models_url_derived_from_v1_base() {
    assert_eq!(
        lm_studio_native_models_url("http://localhost:1234/v1"),
        "http://localhost:1234/api/v0/models"
    );
    // Trailing slash tolerated.
    assert_eq!(
        lm_studio_native_models_url("http://127.0.0.1:1234/v1/"),
        "http://127.0.0.1:1234/api/v0/models"
    );
    // Remote host with path prefix.
    assert_eq!(
        lm_studio_native_models_url("https://lm.example.com/lmstudio/v1"),
        "https://lm.example.com/lmstudio/api/v0/models"
    );
}

/// GH #5055: the `/api/tags` fallback URL is a sibling of `/v1` at the host
/// root. Appending to the `/v1` base would produce `/v1/api/tags` — the
/// exact malformed request LM Studio logs as `Unexpected endpoint or
/// method` (GH #5053).
#[test]
fn ollama_tags_fallback_url_is_host_rooted_not_v1_suffixed() {
    assert_eq!(
        ollama_tags_fallback_url("http://localhost:1234/v1"),
        "http://localhost:1234/api/tags"
    );
    assert_eq!(
        ollama_tags_fallback_url("http://127.0.0.1:1234/v1/"),
        "http://127.0.0.1:1234/api/tags"
    );
    assert_eq!(
        ollama_tags_fallback_url("https://lm.example.com/lmstudio/v1"),
        "https://lm.example.com/lmstudio/api/tags"
    );
    // A host-rooted base (no /v1) is left alone.
    assert_eq!(
        ollama_tags_fallback_url("http://localhost:11434"),
        "http://localhost:11434/api/tags"
    );
    for url in [
        ollama_tags_fallback_url("http://localhost:1234/v1"),
        ollama_tags_fallback_url("http://localhost:1234/v1/"),
    ] {
        assert!(!url.contains("/v1/api/tags"), "malformed probe URL: {url}");
    }
}

#[test]
fn context_window_prefers_loaded_then_max() {
    let resp: LmStudioNativeModelsResponse = serde_json::from_str(
        r#"{"data":[
            {"id":"qwen2.5-7b","state":"loaded","loaded_context_length":4096,"max_context_length":32768},
            {"id":"phi-4","state":"not-loaded","max_context_length":16384}
        ]}"#,
    )
    .unwrap();
    // Loaded model → the runtime-enforced loaded window, NOT the trained max.
    assert_eq!(
        lm_studio_context_window_for(&resp, "qwen2.5-7b"),
        Some(4096)
    );
    // Not-loaded model → declared max as fallback.
    assert_eq!(lm_studio_context_window_for(&resp, "phi-4"), Some(16384));
    // Unknown model id → None (caller falls back to profile default).
    assert_eq!(lm_studio_context_window_for(&resp, "missing"), None);
}

#[test]
fn context_window_treats_zero_and_absent_as_unknown() {
    let resp: LmStudioNativeModelsResponse = serde_json::from_str(
        r#"{"data":[
            {"id":"zeroed","loaded_context_length":0,"max_context_length":0},
            {"id":"bare"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(lm_studio_context_window_for(&resp, "zeroed"), None);
    assert_eq!(lm_studio_context_window_for(&resp, "bare"), None);
}

#[test]
fn normalize_lm_studio_base_url_defaults_scheme_and_v1() {
    assert_eq!(
        normalize_lm_studio_base_url("localhost:1234").as_deref(),
        Some("http://localhost:1234/v1")
    );
}

#[test]
fn normalize_lm_studio_base_url_preserves_existing_v1() {
    assert_eq!(
        normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/").as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );
}

#[test]
fn normalize_lm_studio_base_url_strips_known_endpoint_suffix() {
    assert_eq!(
        normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/chat/completions").as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );
    assert_eq!(
        normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/models").as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );
}
