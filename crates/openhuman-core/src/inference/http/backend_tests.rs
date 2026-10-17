use super::*;

#[test]
fn bare_model_names_route_to_ollama() {
    assert_eq!(provider_string_for("llama3"), "ollama:llama3");
    assert_eq!(provider_string_for("ollama:llama3"), "ollama:llama3");
    assert_eq!(provider_string_for("acme:gpt-4o"), "acme:gpt-4o");
    assert_eq!(provider_string_for("openhuman"), "openhuman");
}

#[test]
fn configured_models_lists_sentinel_and_strips_temperature_suffix() {
    let mut config = Config::default();
    config.default_model = Some("gpt-4o@0.7".to_string());
    config.chat_provider = Some("acme:claude@1".to_string());
    config.local_ai.chat_model_id = "qwen".to_string();
    let ids: Vec<String> = configured_models(&config)
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids[0], "openhuman");
    assert!(ids.contains(&"gpt-4o".to_string()));
    assert!(ids.contains(&"acme:claude".to_string()));
    assert!(ids.contains(&"ollama:qwen".to_string()));
}
