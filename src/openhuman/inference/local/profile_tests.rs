use super::*;

#[test]
fn kind_from_str_loose_accepts_aliases() {
    assert_eq!(
        LocalProviderKind::from_str_loose("ollama"),
        Some(LocalProviderKind::Ollama)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("LM-Studio"),
        Some(LocalProviderKind::LmStudio)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("lm_studio"),
        Some(LocalProviderKind::LmStudio)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("mlx"),
        Some(LocalProviderKind::Mlx)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("mlx-server"),
        Some(LocalProviderKind::Mlx)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("llamacpp"),
        Some(LocalProviderKind::LocalOpenai)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("vllm"),
        Some(LocalProviderKind::LocalOpenai)
    );
    assert_eq!(LocalProviderKind::from_str_loose("unknown"), None);
}

#[test]
fn kind_from_provider_string_parses_prefixes() {
    assert_eq!(
        kind_from_provider_string("ollama:qwen3:14b"),
        Some(LocalProviderKind::Ollama)
    );
    assert_eq!(
        kind_from_provider_string("lmstudio:mistral"),
        Some(LocalProviderKind::LmStudio)
    );
    assert_eq!(
        kind_from_provider_string("mlx:llama-3.1-8b"),
        Some(LocalProviderKind::Mlx)
    );
    assert_eq!(
        kind_from_provider_string("local-openai:qwen2"),
        Some(LocalProviderKind::LocalOpenai)
    );
    assert_eq!(kind_from_provider_string("openai:gpt-4o"), None);
    assert_eq!(kind_from_provider_string("openhuman"), None);
}

#[test]
fn is_local_identifies_local_strings() {
    assert!(is_local_provider_string("ollama:phi3"));
    assert!(is_local_provider_string("mlx:model"));
    assert!(!is_local_provider_string("openai:gpt-4"));
    assert!(!is_local_provider_string("openhuman"));
}

#[test]
fn profiles_have_correct_defaults() {
    let ollama = profile_for_kind(LocalProviderKind::Ollama);
    assert_eq!(ollama.tool_support, ToolSupport::PromptGuided);
    assert_eq!(ollama.default_context_window, Some(8_192));
    assert!(!ollama.supports_responses_api);

    let mlx = profile_for_kind(LocalProviderKind::Mlx);
    assert_eq!(mlx.default_context_window, Some(4_096));
}

#[test]
fn ollama_profile_is_conservative_on_tools() {
    let profile = profile_for_kind(LocalProviderKind::Ollama);
    assert_eq!(profile.tool_support, ToolSupport::PromptGuided);
}

#[test]
fn omlx_kind_and_profile() {
    assert_eq!(
        LocalProviderKind::from_str_loose("omlx"),
        Some(LocalProviderKind::Omlx)
    );
    assert_eq!(
        LocalProviderKind::from_str_loose("omlx-server"),
        Some(LocalProviderKind::Omlx)
    );
    assert_eq!(LocalProviderKind::Omlx.as_str(), "omlx");
    assert_eq!(LocalProviderKind::Omlx.display_name(), "OMLX");
    assert_eq!(
        profile_for_kind(LocalProviderKind::Omlx).default_base_url,
        "http://127.0.0.1:8000/v1"
    );
    assert_eq!(
        kind_from_provider_string("omlx:my-model"),
        Some(LocalProviderKind::Omlx)
    );
    assert!(is_local_provider_string("omlx:my-model"));
}
