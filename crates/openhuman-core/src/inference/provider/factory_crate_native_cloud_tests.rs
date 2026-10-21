fn create_chat_model_routes_plain_bearer_cloud_slug_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    // DeepSeek is a built-in chat-completions-only Bearer provider: no
    // `/v1/responses` fallback and no codex-oauth, so it is wire-equivalent and
    // flips crate-native.
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    config.chat_provider = Some("deepseek:deepseek-reasoner".to_string());
    let (model, model_id) = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
    .expect("bearer cloud create_chat_model must build");
    assert_eq!(model_id, "deepseek-reasoner");
    let profile = model
        .profile()
        .expect("crate-native cloud model exposes a profile");
    assert_eq!(profile.provider.as_deref(), Some("deepseek"));
    // A generic cloud model keeps native tool calling + vision on (unlike the
    // local runtimes), so this is the crate `OpenAiModel` default profile.
    assert!(profile.tool_calling);
}

#[test]
fn turn_model_route_metadata_uses_post_remap_cloud_model() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    config.chat_provider = Some("deepseek:hint:chat".to_string());

    let (_model, provider, resolved_model) = create_turn_chat_model_with_native_tools_and_route(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        "hint:chat",
        0.7,
        true,
        &EgressLedger::none(),
    )
    .expect("abstract BYOK tier must build");

    assert_eq!(provider, "deepseek");
    assert_eq!(resolved_model, "deepseek-chat");
}

#[test]
fn explicit_cloud_provider_string_routes_to_crate_native_model() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    let (model, model_id) = create_chat_model_from_string_with_model_id(
        "chat",
        "deepseek:deepseek-reasoner",
        &config,
        0.7,
    )
    .expect("explicit cloud model must build");
    assert_eq!(model_id, "deepseek-reasoner");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("deepseek")
    );
}

#[test]
fn create_chat_model_routes_anthropic_auth_cloud_slug_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    // Anthropic-auth cloud slugs are always wire-equivalent (their endpoints have
    // no `/v1/responses`, so the host's dormant fallback is behavior-neutral).
    let mut config = Config::default();
    config
        .cloud_providers
        .push(anthropic_entry("p_anth", "anthropic"));
    config.chat_provider = Some("anthropic:claude-sonnet-4-6".to_string());
    let (model, model_id) = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
    .expect("anthropic cloud create_chat_model must build");
    assert_eq!(model_id, "claude-sonnet-4-6");
    assert_eq!(
        model.profile().and_then(|p| p.provider.as_deref()),
        Some("anthropic")
    );
    // The native Messages API adapter, not the OpenAI-compat flavour: its
    // cache identity is `anthropic:<base>:<model>`, while the compat client
    // would report the provider slug with an OpenAI-shaped identity. Only the
    // native adapter places `cache_control` breakpoints — Anthropic's compat
    // endpoint documents prompt caching as unsupported.
    let identity = model.cache_identity().expect("anthropic identifies itself");
    assert!(
        identity.starts_with("anthropic:https://api.anthropic.com/v1:claude-sonnet-4-6"),
        "expected the native Messages adapter, got identity {identity}"
    );
    assert!(model
        .profile()
        .is_some_and(|p| p.tool_calling && p.streaming));
}

/// Text mode (prompt-guided tools) is only implemented on the Chat Completions
/// adapter, so an Anthropic slug asked for `native_tools = false` keeps the
/// compat client rather than a native adapter that would ignore the request.
#[test]
fn anthropic_auth_cloud_slug_in_text_mode_keeps_the_compat_client() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config
        .cloud_providers
        .push(anthropic_entry("p_anth", "anthropic"));
    let (model, model_id) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "anthropic:claude-sonnet-4-6",
        &config,
        false,
        &EgressLedger::none(),
    )
    .expect("anthropic slug resolves")
    .expect("build");
    assert_eq!(model_id, "claude-sonnet-4-6");
    let identity = model.cache_identity().unwrap_or_default();
    assert!(
        !identity.starts_with("anthropic:https://"),
        "text mode must not route to the native adapter, got {identity}"
    );
    assert!(model.profile().is_some_and(|p| !p.tool_calling));
}

#[test]
fn configured_openhuman_jwt_slug_routes_to_managed_chat_model() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));
    // A retired tier slug after the managed slug is a role alias: it runs on
    // the managed default, never reaches the backend verbatim.
    config.chat_provider = Some("openhuman:reasoning-v1".to_string());

    let (model, model_id) =
        try_create_cloud_slug_chat_model("chat", &config, &EgressLedger::none())
            .expect("configured OpenhumanJwt slug should be recognized")
            .expect("managed model should build");

    assert_eq!(model_id, crate::config::MODEL_MANAGED_DEFAULT);

    // A concrete catalog id is forwarded verbatim.
    config.chat_provider = Some("openhuman:openrouter/deepseek/deepseek-v4-pro".to_string());
    let (_, pinned_id) = try_create_cloud_slug_chat_model("chat", &config, &EgressLedger::none())
        .expect("configured OpenhumanJwt slug should be recognized")
        .expect("managed model should build");
    assert_eq!(pinned_id, "openrouter/deepseek/deepseek-v4-pro");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("managed"),
        "OpenhumanJwt must use the crate-native managed backend model"
    );
}

#[tokio::test]
async fn openhuman_jwt_slug_discloses_pinned_model() {
    use crate::core::events::DomainEvent;
    use crate::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "egress-jwt-pinned-marker-v1";
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));
    let provider = format!("openhuman:{marker}");
    let _ = try_create_cloud_slug_chat_model_from_string(
        "chat",
        &provider,
        &config,
        &EgressLedger::none(),
    )
    .expect("configured OpenhumanJwt slug should be recognized")
    .expect("managed model should build");

    let sentinel = "egress-jwt-pinned-sentinel-end";
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });

    let mut count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    assert_eq!(descriptor.provider_slug, "openhuman");
                    assert!(matches!(descriptor.reason, EgressReason::Inference));
                    count += 1;
                } else if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
    assert_eq!(
        count, 1,
        "JWT construction must disclose its pinned model once"
    );
}

#[tokio::test]
async fn native_claude_turn_routes_disclose_pinned_models() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let configured_sdk = "egress-sdk-configured-marker";
    let pinned_sdk = "egress-sdk-pinned-marker";
    let mut sdk_config = Config::default();
    sdk_config.chat_provider = Some(format!("claude_agent_sdk:{configured_sdk}"));
    create_turn_chat_model(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &sdk_config,
        pinned_sdk,
        0.0,
    )
    .expect("Claude Agent SDK turn model should build");

    let configured_code = "egress-code-configured-marker";
    let pinned_code = "egress-code-pinned-marker";
    let mut code_config = Config::default();
    code_config.chat_provider = Some(format!("claude-code:{configured_code}"));
    // Egress is disclosed once the effective model is selected, before the
    // environment probe. The test therefore remains valid on hosts without the
    // Claude Code CLI.
    let _ = create_turn_chat_model(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &code_config,
        pinned_code,
        0.0,
    );

    let sentinel = "egress-native-claude-sentinel-end";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });

    let mut sdk_count = 0usize;
    let mut code_count = 0usize;
    let mut configured_count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                match descriptor.service.as_str() {
                    service if service == pinned_sdk => {
                        assert_eq!(descriptor.provider_slug, "claude_agent_sdk");
                        sdk_count += 1;
                    }
                    service if service == pinned_code => {
                        assert_eq!(descriptor.provider_slug, "claude-code");
                        code_count += 1;
                    }
                    service if service == configured_sdk || service == configured_code => {
                        configured_count += 1;
                    }
                    service if service == sentinel => break,
                    _ => {}
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }

    assert_eq!(
        sdk_count, 1,
        "SDK route must disclose its pinned model once"
    );
    assert_eq!(
        code_count, 1,
        "Claude Code route must disclose its pinned model once"
    );
    assert_eq!(
        configured_count, 0,
        "native Claude routes must not disclose stale configured models"
    );
}

#[test]
fn openhuman_jwt_slug_preserves_forced_text_mode() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));

    let (model, _) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "openhuman:reasoning-v1",
        &config,
        false,
        &EgressLedger::none(),
    )
    .expect("configured OpenhumanJwt slug should be recognized")
    .expect("managed model should build");

    let profile = model
        .profile()
        .expect("managed model should expose its effective capabilities");
    assert!(!profile.tool_calling);
    assert!(!profile.parallel_tool_calls);
    assert!(!profile.streaming_tool_chunks);
}

#[test]
fn openhuman_jwt_slug_without_model_preserves_managed_role_tier() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));

    let (_model, model_id) = try_create_cloud_slug_chat_model_from_string(
        "summarization",
        "openhuman:",
        &config,
        &EgressLedger::none(),
    )
    .expect("configured OpenhumanJwt slug should be recognized")
    .expect("managed model should build");

    assert_eq!(model_id, crate::config::MODEL_MANAGED_DEFAULT);
}

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
            .is_none()
    );
    let mut local = Config::default();
    local.chat_provider = Some("ollama:qwen2.5".to_string());
    assert!(try_create_cloud_slug_chat_model("chat", &local, &EgressLedger::none()).is_none());
    let mut unconfigured = Config::default();
    unconfigured.chat_provider = Some("deepseek:deepseek-chat".to_string());
    assert!(
        try_create_cloud_slug_chat_model("chat", &unconfigured, &EgressLedger::none()).is_none()
    );
}
