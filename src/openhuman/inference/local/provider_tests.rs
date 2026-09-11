use super::*;

#[test]
fn normalize_provider_accepts_lm_studio_aliases() {
    assert_eq!(normalize_provider("lmstudio"), "lm_studio");
    assert_eq!(normalize_provider("lm-studio"), "lm_studio");
    assert_eq!(normalize_provider("LM_Studio"), "lm_studio");
}

#[test]
fn normalize_provider_falls_back_to_ollama() {
    assert_eq!(normalize_provider(""), "ollama");
    assert_eq!(normalize_provider("unknown"), "ollama");
}

#[test]
fn normalize_provider_keeps_omlx() {
    assert_eq!(normalize_provider("omlx"), "omlx");
    assert_eq!(normalize_provider("omlx-server"), "omlx");
    assert_eq!(normalize_provider("OMLX"), "omlx");
}

#[test]
fn endpoint_is_openai_v1_detects_v1_root() {
    assert!(endpoint_is_openai_v1("http://localhost:1234/v1"));
    assert!(endpoint_is_openai_v1("http://localhost:1234/v1/"));
    assert!(endpoint_is_openai_v1("https://box.local:1234/openai/v1"));
    // Genuine Ollama base is host-rooted with no /v1 path.
    assert!(!endpoint_is_openai_v1("http://localhost:11434"));
    assert!(!endpoint_is_openai_v1("http://localhost:11434/"));
    // A `/v1` embedded mid-path is not the OpenAI root.
    assert!(!endpoint_is_openai_v1("http://localhost:11434/v1/models"));
}

#[test]
fn model_discovery_api_uses_tags_for_genuine_ollama() {
    // Ollama slug on its host-rooted native base -> /api/tags.
    assert_eq!(
        model_discovery_api("ollama", "http://localhost:11434"),
        ModelDiscoveryApi::OllamaTags
    );
    assert_eq!(
        model_discovery_api("", "http://localhost:11434"),
        ModelDiscoveryApi::OllamaTags
    );
}

#[test]
fn model_discovery_api_uses_v1_models_for_openai_compatible() {
    // Explicit LM Studio / OMLX slugs are OpenAI-compatible.
    assert_eq!(
        model_discovery_api("lm_studio", "http://localhost:1234/v1"),
        ModelDiscoveryApi::OpenAiModels
    );
    assert_eq!(
        model_discovery_api("omlx", "http://localhost:8080/v1"),
        ModelDiscoveryApi::OpenAiModels
    );
    // The #5053 case: a custom BYOK OpenAI-compatible endpoint on localhost
    // whose provider tag still defaults to `ollama` must NOT be probed with
    // /api/tags — the `/v1` endpoint type wins.
    assert_eq!(
        model_discovery_api("ollama", "http://localhost:1234/v1"),
        ModelDiscoveryApi::OpenAiModels
    );
    assert_eq!(
        model_discovery_api("custom-byok", "http://localhost:1234/v1"),
        ModelDiscoveryApi::OpenAiModels
    );
}
