use super::*;

#[test]
fn try_create_cloud_slug_flips_openai_but_declines_non_cloud() {
    let _guard = crate::inference::inference_test_guard();
    // `openai` (API-key Bearer, no codex OAuth) now flips crate-native on Chat
    // Completions — the legacy `/v1/responses` fallback is not replicated.
    let mut openai = Config::default();
    openai.cloud_providers.push(openai_entry("p_oai", "openai"));
    openai.chat_provider = Some("openai:gpt-4o-mini".to_string());
    let (model, model_id) =
        try_create_cloud_slug_chat_model("chat", &openai, &EgressLedger::none())
            .await
            .expect("openai should flip crate-native")
            .expect("build");
    assert_eq!(model_id, "gpt-4o-mini");
    assert_eq!(
        model.profile().and_then(|p| p.provider.as_deref()),
        Some("openai")
    );

    // Managed (default), local runtimes, and unconfigured slugs are not cloud
    // slugs — they decline and fall through to their own paths.
    assert!(
        try_create_cloud_slug_chat_model("chat", &Config::default(), &EgressLedger::none())
            .await
            .is_none()
    );
    let mut local = Config::default();
    local.chat_provider = Some("ollama:qwen2.5".to_string());
    assert!(
        try_create_cloud_slug_chat_model("chat", &local, &EgressLedger::none())
            .await
            .is_none()
    );
    let mut unconfigured = Config::default();
    unconfigured.chat_provider = Some("deepseek:deepseek-chat".to_string());
    assert!(
        try_create_cloud_slug_chat_model("chat", &unconfigured, &EgressLedger::none())
            .await
            .is_none()
    );
}
