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
    use crate::openhuman::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    // Unique model marker so the process-wide bus can't confuse a concurrent
    // test's managed event with ours. `heartbeat` has no managed tier and
    // resolves to the managed backend, so `default_model` flows through verbatim.
    let marker = "egress-managed-realpath-marker-v1";
    let mut config = Config::default();
    config.default_model = Some(marker.to_string());
    let _ = create_chat_model_with_model_id("heartbeat", &config, 0.7);

    // Bound the drain with a unique sentinel published AFTER our construction.
    let sentinel = "egress-managed-sentinel-end";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
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
}

/// Real-path smoke (privacy epic S2, #4436): a LOCAL runtime construction on the
/// crate-native ChatModel path must NOT publish an `ExternalTransferPending`
/// (nothing leaves the device — it is disclosed as non-external, no event).
#[tokio::test]
async fn create_chat_model_local_runtime_does_not_emit_egress_realpath() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::openhuman::security::egress::EgressDescriptor;
    use std::time::Duration;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let local_marker = "egress-local-realpath-marker";
    let mut config = Config::default();
    config.chat_provider = Some(format!("ollama:{local_marker}"));
    let _ = create_chat_model_with_model_id("chat", &config, 0.7);

    // Sentinel bounds the drain; if the local marker ever appears as an external
    // transfer before it, the local-suppression contract is broken.
    let sentinel = "egress-local-sentinel-end";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
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

#[test]
fn local_chat_still_routes_background_roles_to_the_managed_backend() {
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
        "heartbeat",
        "learning",
        "subconscious",
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

#[test]
fn local_chat_role_is_returned_verbatim_and_never_falls_back() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "ollama:gemma3:1b",
        "an explicitly configured local chat route must be honoured verbatim"
    );
}

#[test]
fn explicit_background_route_overrides_the_cloud_fallback() {
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

#[test]
fn a_readable_profile_with_no_stored_key_is_treated_as_missing_credentials() {
    // The common BYOK-with-no-key shape: the auth profile reads fine, it just
    // has nothing for this slug, so the lookup succeeds with an empty string.
    // Without an emptiness check the client would be built with a blank bearer
    // and the user would get a raw 401 from the provider instead of guidance.
    let _guard = crate::openhuman::inference::inference_test_guard();
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    let err = create_test_chat_model_from_string("vision", "openai:gpt-4o", &config)
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
        create_test_chat_model_from_string("vision", "openai:gpt-4o", &config).is_ok(),
        "an explicitly routed provider must still build without a stored key"
    );
}

#[test]
fn implicit_cloud_fallback_is_claimed_only_when_the_role_has_no_route_of_its_own() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // Unset background route: the role genuinely landed on the cloud because
    // the local chat model cannot serve it, so the explanation applies.
    assert!(role_uses_implicit_cloud_fallback("vision", &config));
    // The literal "cloud" is the same "route me wherever the cloud is" intent.
    config.embeddings_provider = Some("cloud".to_string());
    assert!(role_uses_implicit_cloud_fallback("embeddings", &config));
    // Whitespace is not a configured route.
    config.memory_provider = Some("   ".to_string());
    assert!(role_uses_implicit_cloud_fallback("memory", &config));

    // Explicitly routed to a cloud slug: a credential failure here is about
    // that route, not about the local chat model, so it must not be described
    // as a fallback.
    config.vision_provider = Some("anthropic:claude-3-5-sonnet-latest".to_string());
    assert!(!role_uses_implicit_cloud_fallback("vision", &config));

    // Chat-tier roles are never described as cloud fallbacks, routed or not.
    for role in ["chat", "reasoning", "coding"] {
        assert!(!role_uses_implicit_cloud_fallback(role, &config));
    }
}

#[test]
fn cloud_fallback_roles_match_the_roles_provider_for_role_actually_falls_back() {
    // `factory_tests` is a child module of `factory`, so `super` is `factory`,
    // not `provider` — reach the sibling module by its crate path.
    use crate::openhuman::inference::provider::fallback_diagnostics::role_falls_back_to_cloud;
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // The diagnostics module carries its own role list; if the two drift, users
    // get either a missing explanation or one that names the wrong knob.
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "heartbeat",
        "learning",
        "subconscious",
        "agentic",
        "burst",
    ] {
        assert!(
            role_falls_back_to_cloud(role),
            "'{role}' falls back in provider_for_role but is missing from CLOUD_FALLBACK_ROLES"
        );
    }
    for role in ["chat", "reasoning", "coding"] {
        assert!(
            !role_falls_back_to_cloud(role),
            "'{role}' is a chat-tier role and must not be described as a cloud fallback"
        );
    }
}

#[test]
fn a_routed_turn_resolves_its_roles_to_the_callers_endpoint() {
    use crate::openhuman::config::schema::EPHEMERAL_ROUTE_SLUG;
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

#[test]
fn a_routed_turn_authenticates_with_the_callers_bearer() {
    use crate::openhuman::config::schema::EPHEMERAL_ROUTE_SLUG;
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    // Read straight off the config: there is no auth profile on disk for a slug
    // that exists only in this in-memory copy, so the stored-profile lookup
    // would come back empty and the turn would 401 several layers later.
    assert_eq!(
        lookup_key_for_slug(EPHEMERAL_ROUTE_SLUG, &config).expect("resolves"),
        "mdl-token"
    );
}

#[test]
fn the_routes_bearer_is_not_offered_to_any_other_slug() {
    // The containment that makes reusing one config field safe: a background
    // role that fell through to some other cloud slug must not be handed the
    // caller's credential.
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    for slug in ["openrouter", "anthropic", "openai"] {
        let key = lookup_key_for_slug(slug, &config).expect("resolves");
        assert_ne!(key, "mdl-token", "slug '{slug}' must not see the route key");
    }
}

#[test]
fn the_route_resolves_to_a_provider_the_factory_can_build() {
    // `resolve_cloud_slug` is where a missing `cloud_providers` entry or an
    // empty model turns into an error. Building the model proves the entry
    // `apply` registered is complete enough to be used, not merely present.
    let config = routed_config("http://127.0.0.1:41234/openai", "mdl-token", "x/y");
    let (_, model_id) = create_test_chat_model_from_string(
        "coding",
        &provider_for_role("coding", &config),
        &config,
    )
    .expect("the per-call route builds a chat model");
    assert_eq!(model_id, "x/y");
}
