use super::*;

/// Real-path smoke (privacy epic S2, #4436): the crate-native ChatModel path
/// `create_chat_model_with_model_id` — the path production agent turns use
/// post-#4784 — must emit EXACTLY ONE egress descriptor for a managed-backend
/// (external) construction. Regression guard for the gap where emit lived only
/// on the legacy `Provider` path, so the default managed turn disclosed nothing.
#[tokio::test]
async fn create_chat_model_managed_emits_exactly_one_egress_realpath() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    super::install_test_managed_provider();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    // Unique model marker so the process-wide bus can't confuse a concurrent
    // test's managed event with ours. `scratch` has no managed tier and
    // resolves to the managed backend, so `default_model` flows through verbatim.
    let marker = "egress-managed-realpath-marker-v1";
    let mut config = Config::default();
    config.default_model = Some(marker.to_string());
    super::with_library_test_context_async(|| async {
        let _ = create_chat_model_with_model_id_in(
            &crate::core::runtime::DispatchCtx::ambient(),
            "scratch",
            &config,
            0.7,
        )
        .await;
    })
    .await;

    // Bound the drain with a unique sentinel published AFTER our construction.
    let sentinel = "egress-managed-sentinel-end";
    BUS.publish(DomainEvent::ExternalTransferPending {
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
                    assert!(descriptor.is_external, "managed backend is external");
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
        "managed inference via create_chat_model_with_model_id must emit EXACTLY ONE egress descriptor (no miss, no double)"
    );
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

/// Real-path smoke (privacy epic S2, #4436): a LOCAL runtime construction on the
/// crate-native ChatModel path must NOT publish an `ExternalTransferPending`
/// (nothing leaves the device — it is disclosed as non-external, no event).
#[tokio::test]
async fn create_chat_model_local_runtime_does_not_emit_egress_realpath() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let local_marker = "egress-local-realpath-marker";
    let mut config = Config::default();
    config.chat_provider = Some(format!("ollama:{local_marker}"));
    let _ = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
    .await;

    // Sentinel bounds the drain; if the local marker ever appears as an external
    // transfer before it, the local-suppression contract is broken.
    let sentinel = "egress-local-sentinel-end";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });

    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                assert_ne!(
                    descriptor.service, local_marker,
                    "local runtime must NOT publish ExternalTransferPending"
                );
                if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
}

// ─── #5146 §2.1: local chat + background workloads ────────────────────────────
//
// The fix for #5146 §2.1 is an *explanation* change, not a routing change. These
// pin the routing so a future "never fall back" refactor cannot silently break
// local-chat + managed-subscription users without a failing test.

#[tokio::test]
async fn local_chat_still_routes_background_roles_to_the_managed_backend() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // Every background role keeps falling through to the managed backend: they
    // run tier-specific models a local runtime does not serve, and the user's
    // subscription is what pays for them.
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "agentic",
        "burst",
    ] {
        assert_eq!(
            provider_for_role(role, &config),
            "openhuman",
            "role '{role}' must keep falling back to the managed backend when chat is local"
        );
    }
}

#[tokio::test]
async fn local_chat_role_is_returned_verbatim_and_never_falls_back() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "ollama:gemma3:1b",
        "an explicitly configured local chat route must be honoured verbatim"
    );
}

#[tokio::test]
async fn explicit_background_route_overrides_the_cloud_fallback() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());
    config.vision_provider = Some("ollama:llava:7b".to_string());

    // The remedy the new diagnostics point users at must actually work.
    assert_eq!(
        provider_for_role("vision", &config),
        "ollama:llava:7b",
        "setting vision_provider must take precedence over the cloud fallback"
    );
}

#[tokio::test]
async fn a_readable_profile_with_no_stored_key_is_treated_as_missing_credentials() {
    // The common BYOK-with-no-key shape: the auth profile reads fine, it just
    // has nothing for this slug, so the lookup succeeds with an empty string.
    // Without an emptiness check the client would be built with a blank bearer
    // and the user would get a raw 401 from the provider instead of guidance.
    let _guard = crate::inference::inference_test_guard();
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    let err = create_test_chat_model_from_string("vision", "openai:gpt-4o", &config)
        .await
        .err()
        .expect("a slug with no stored key must not build a client");
    let message = err.to_string();

    assert!(
        message.contains("No usable credentials for 'openai'"),
        "expected the actionable guidance, got: {message}"
    );
    // It is a genuine implicit fallback here (vision has no route of its own),
    // so the local chat model that caused it is named.
    assert!(
        message.contains("ollama:gemma3:1b"),
        "expected the local chat model to be named, got: {message}"
    );

    // Scope: an explicitly routed provider is NOT failed at construction time
    // for a missing key. Callers build such models to probe or describe a
    // provider before a key is saved, so only the implicit-fallback path (the
    // one this diagnostic exists for) turns a blank key into an error.
    config.vision_provider = Some("openai:gpt-4o".to_string());
    assert!(
        create_test_chat_model_from_string("vision", "openai:gpt-4o", &config)
            .await
            .is_ok(),
        "an explicitly routed provider must still build without a stored key"
    );
}

#[tokio::test]
async fn a_failed_credential_lookup_does_not_echo_its_underlying_error() {
    // The lookup error can carry stored-credential content, so the user-facing
    // error is the guidance alone; the routing shape goes to the log. The
    // read-only (doctor) lookup fails on an unreadable profile store.
    let _guard = crate::inference::inference_test_guard();
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openai:gpt-4o".to_string());
    std::fs::write(
        tmp.path().join("auth-profiles.json"),
        "{ not json sk-leak-marker",
    )
    .expect("corrupt profile store");

    let err = match super::super::inspection::create_chat_model_with_model_id_in_for_inspection(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.0,
    )
    .await
    {
        Ok(_) => panic!("an unreadable credential store must not build a client"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.contains("No usable credentials for 'openai'"),
        "expected the actionable guidance, got: {message}"
    );
    assert!(
        !message.contains("underlying error") && !message.contains("leak-marker"),
        "the lookup error leaked into the user-facing message: {message}"
    );
}

#[tokio::test]
async fn implicit_cloud_fallback_is_claimed_only_when_the_role_has_no_route_of_its_own() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // Unset background route: the role genuinely landed on the cloud because
    // the local chat model cannot serve it, so the explanation applies.
    assert!(fallback_diagnostics_for_role("vision", &config).is_some());
    // The literal "cloud" is the same "route me wherever the cloud is" intent.
    config.embeddings_provider = Some("cloud".to_string());
    assert!(fallback_diagnostics_for_role("embeddings", &config).is_some());
    // Whitespace is not a configured route.
    config.memory_provider = Some("   ".to_string());
    assert!(fallback_diagnostics_for_role("memory", &config).is_some());

    // Explicitly routed to a cloud slug: a credential failure here is about
    // that route, not about the local chat model, so it must not be described
    // as a fallback.
    config.vision_provider = Some("anthropic:claude-3-5-sonnet-latest".to_string());
    assert!(fallback_diagnostics_for_role("vision", &config).is_none());

    // Chat-tier roles are never described as cloud fallbacks, routed or not.
    for role in ["chat", "reasoning", "coding"] {
        assert!(fallback_diagnostics_for_role(role, &config).is_none());
    }
}

#[tokio::test]
async fn cloud_fallback_roles_match_the_roles_provider_for_role_actually_falls_back() {
    let router = crate::inference::provider::factory::workload_router();

    // The native router's metadata is the source of truth for roles that may
    // fall back to the primary cloud provider.
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "agentic",
        "burst",
    ] {
        assert!(
            router.cloud_fallback_role(role).is_some(),
            "'{role}' falls back in provider_for_role but is missing native fallback metadata"
        );
    }
    for role in ["chat", "reasoning", "coding"] {
        assert!(
            router.cloud_fallback_role(role).is_none(),
            "'{role}' is a chat-tier role and must not be described as a cloud fallback"
        );
    }
}

#[tokio::test]
async fn a_routed_turn_resolves_its_roles_to_the_callers_endpoint() {
    use crate::config::schema::EPHEMERAL_ROUTE_SLUG;
    let config = routed_config(
        "http://127.0.0.1:41234/openai",
        "mdl-token",
        "anthropic/claude-sonnet-4",
    );
    let expected = format!("{EPHEMERAL_ROUTE_SLUG}:anthropic/claude-sonnet-4");
    for role in ["chat", "reasoning", "coding", "agentic"] {
        assert_eq!(
            provider_for_role(role, &config),
            expected,
            "role '{role}' must resolve to the per-call route"
        );
    }
}

#[tokio::test]
async fn a_routed_turn_authenticates_with_the_callers_bearer() {
    use crate::config::schema::EPHEMERAL_ROUTE_SLUG;
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    // Read straight off the config: there is no auth profile on disk for a slug
    // that exists only in this in-memory copy, so the stored-profile lookup
    // would come back empty and the turn would 401 several layers later.
    assert_eq!(
        lookup_key_for_slug(EPHEMERAL_ROUTE_SLUG, &config)
            .await
            .expect("resolves"),
        "mdl-token"
    );
}

#[tokio::test]
async fn the_routes_bearer_is_not_offered_to_any_other_slug() {
    // The containment that makes reusing one config field safe: a background
    // role that fell through to some other cloud slug must not be handed the
    // caller's credential.
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    for slug in ["openrouter", "anthropic", "openai"] {
        let key = lookup_key_for_slug(slug, &config).await.expect("resolves");
        assert_ne!(key, "mdl-token", "slug '{slug}' must not see the route key");
    }
}

#[tokio::test]
async fn the_route_resolves_to_a_provider_the_factory_can_build() {
    // `resolve_cloud_slug` is where a missing `cloud_providers` entry or an
    // empty model turns into an error. Building the model proves the entry
    // `apply` registered is complete enough to be used, not merely present.
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    let (_, model_id) = create_test_chat_model_from_string(
        "coding",
        &provider_for_role("coding", &config),
        &config,
    )
    .await
    .expect("the per-call route builds a chat model");
    assert_eq!(model_id, "x/y");
}

#[tokio::test]
async fn a_failed_lookup_keeps_only_its_expiry_signal() {
    // The doctor reports "requires refresh" and chat routes a Codex expiry to
    // the reconnect copy, both from these fixed phrases; the detail beside
    // them (a refresh response's body) stays out.
    use super::super::cloud_slug::lookup_failure;
    let guidance = || "No usable credentials for 'openai'.".to_string();
    for (error, signal) in [
        (
            anyhow::anyhow!("OpenAI OAuth credential requires refresh"),
            "requires refresh",
        ),
        (
            anyhow::anyhow!(
                "Codex authentication token is expired — refresh failed: leak-marker. Please reconnect Codex."
            ),
            "Codex authentication token is expired",
        ),
    ] {
        let message = lookup_failure(&error, guidance()).to_string();
        assert!(message.contains(signal), "{message}");
        assert!(message.contains("No usable credentials"), "{message}");
        assert!(!message.contains("leak-marker"), "{message}");
    }
    let message = lookup_failure(
        &anyhow::anyhow!("store read failed: leak-marker"),
        guidance(),
    );
    assert_eq!(message.to_string(), guidance());
}
