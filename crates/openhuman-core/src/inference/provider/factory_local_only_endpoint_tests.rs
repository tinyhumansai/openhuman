use super::*;
use crate::config::test_env::EnvVarGuard;
use crate::config::PrivacyMode;
use crate::inference::provider::factory::access_gates::local_only_violation;

// Local-only privacy mode means the prompt stays on this machine. Two routes
// named "local" break that: an Ollama `-cloud` tag, which the local daemon
// proxies to ollama.com, and a local-runtime prefix whose endpoint is another
// host (it gets the prompt and the `local_ai` bearer key). Only loopback counts
// as local; a LAN address is another machine.

/// Environment overrides that take precedence over `local_ai.base_url`.
const ENDPOINT_ENV: &[&str] = &[
    "OPENHUMAN_LOCAL_INFERENCE_URL",
    "LOCAL_OPENAI_URL",
    "OPENHUMAN_LM_STUDIO_BASE_URL",
    "LM_STUDIO_BASE_URL",
    "OPENHUMAN_OLLAMA_BASE_URL",
    "OLLAMA_HOST",
    "MLX_SERVER_URL",
    "OMLX_SERVER_URL",
];

fn local_ai_at(base_url: &str) -> Config {
    let mut config = Config::default();
    config.local_ai.base_url = Some(base_url.to_string());
    config
}

#[tokio::test]
async fn local_only_refuses_ollama_cloud_proxied_models() {
    for provider in [
        "ollama:gpt-oss:120b-cloud",
        "ollama:kimi-k2:1t-cloud@0.2",
        "ollama:deepseek-v3.1:671B-Cloud",
    ] {
        assert!(
            local_only_violation(PrivacyMode::LocalOnly, provider).is_some(),
            "LocalOnly permitted the cloud-proxied model {provider}"
        );
    }
    assert_eq!(
        local_only_violation(PrivacyMode::LocalOnly, "ollama:llama3.1:8b"),
        None
    );
    assert_eq!(
        local_only_violation(PrivacyMode::Standard, "ollama:gpt-oss:120b-cloud"),
        None
    );
}

#[tokio::test]
async fn local_only_refuses_a_local_runtime_pointed_at_another_host() {
    let _guard = crate::inference::inference_test_guard();
    let _env = EnvVarGuard::locked_unset_many(ENDPOINT_ENV);
    let _mode = crate::security::live_policy::test_privacy_scope(PrivacyMode::LocalOnly);
    for (provider, base_url) in [
        ("local-openai:m", "https://gpu.example.invalid/v1"),
        ("lmstudio:m", "http://192.168.1.20:1234/v1"),
        ("ollama:m", "http://gpu.example.invalid:11434"),
    ] {
        let result =
            create_test_chat_model_from_string("chat", provider, &local_ai_at(base_url)).await;
        let error = match result {
            Ok(_) => panic!("LocalOnly built {provider} against {base_url}"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("Local-only"),
            "{provider} at {base_url}: unexpected error {error}"
        );
    }
    for base_url in [
        "http://127.0.0.1:1234/v1",
        "http://localhost:11434/v1",
        "http://[::1]:8080/v1",
    ] {
        assert!(
            create_test_chat_model_from_string("chat", "local-openai:m", &local_ai_at(base_url))
                .await
                .is_ok(),
            "LocalOnly refused the loopback endpoint {base_url}"
        );
    }
}

#[tokio::test]
async fn a_local_runtime_pointed_at_another_host_is_recorded_as_external_egress() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = EnvVarGuard::locked_unset_many_async(ENDPOINT_ENV).await;
    let _mode = crate::security::live_policy::test_privacy_scope(PrivacyMode::Standard);
    crate::core::bus::init().await.expect("bus init");
    let mut rx = BUS.get().unwrap().receiver();

    let marker = "egress-remote-local-runtime-marker";
    let mut config = local_ai_at("https://gpu.example.invalid/v1");
    config.chat_provider = Some(format!("local-openai:{marker}"));
    let _ = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
    .await;

    let sentinel = "egress-remote-local-runtime-sentinel";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    break;
                }
                assert_ne!(
                    descriptor.service, sentinel,
                    "a remote local-runtime endpoint was recorded as local egress"
                );
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
}

/// A request may name a different model than the route was built for (a flow
/// node's pinned model): an Ollama route built for a local tag must still
/// refuse a `-cloud` tag at request time under local-only.
#[tokio::test]
async fn local_only_refuses_a_cloud_tag_named_per_request() {
    use tinyinference_llm::model::ModelRequest;
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = EnvVarGuard::locked_unset_many_async(ENDPOINT_ENV).await;
    let _mode = crate::security::live_policy::test_privacy_scope(PrivacyMode::LocalOnly);
    let (model, _) = create_test_chat_model_from_string(
        "chat",
        "ollama:llama3",
        &local_ai_at("http://127.0.0.1:9/v1"),
    )
    .await
    .expect("a local Ollama tag builds under local-only");

    let error = model
        .invoke(
            &(),
            ModelRequest::new(Vec::new()).with_model("gpt-oss:120b-cloud"),
        )
        .await
        .expect_err("a per-request cloud tag must be refused before dispatch");
    assert!(error.to_string().contains("Local-only"), "{error}");
}

/// The privacy mode is live: a local runtime built for another host under
/// `Standard` must still stop there once the mode is local-only, before any
/// request leaves (every local runtime, not only Ollama).
#[tokio::test]
async fn local_only_set_after_construction_refuses_a_remote_local_runtime_at_dispatch() {
    use tinyinference_llm::model::ModelRequest;
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = EnvVarGuard::locked_unset_many_async(ENDPOINT_ENV).await;
    let _standard = crate::security::live_policy::test_privacy_scope(PrivacyMode::Standard);
    let (model, _) = create_test_chat_model_from_string(
        "chat",
        "local-openai:m",
        &local_ai_at("https://gpu.example.invalid/v1"),
    )
    .await
    .expect("Standard builds a remote local runtime");

    let _local_only = crate::security::live_policy::test_privacy_scope(PrivacyMode::LocalOnly);
    let invoked = model.invoke(&(), ModelRequest::new(Vec::new())).await;
    let streamed = model.stream(&(), ModelRequest::new(Vec::new())).await;
    for error in [
        invoked
            .map(|_| ())
            .expect_err("invoke is refused before dispatch"),
        streamed
            .map(|_| ())
            .expect_err("stream is refused before dispatch"),
    ] {
        assert!(error.to_string().contains("Local-only"), "{error}");
    }
}

/// Outside local-only a per-request `-cloud` tag is allowed, but the prompt
/// goes to ollama.com: it is disclosed as external egress for the model the
/// request names, not the local tag the route was built for.
#[tokio::test]
async fn a_cloud_tag_named_per_request_is_recorded_as_external_egress() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;
    use tinyinference_llm::model::ModelRequest;

    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = EnvVarGuard::locked_unset_many_async(ENDPOINT_ENV).await;
    let _mode = crate::security::live_policy::test_privacy_scope(PrivacyMode::Standard);
    crate::core::bus::init().await.expect("bus init");
    let mut rx = BUS.get().unwrap().receiver();

    let (model, _) = create_test_chat_model_from_string(
        "chat",
        "ollama:llama3",
        &local_ai_at("http://127.0.0.1:9/v1"),
    )
    .await
    .expect("a local Ollama tag builds");
    let marker = "egress-per-request-marker-cloud";
    // Nothing listens on port 9: the request fails after the guard ran.
    let _ = model
        .invoke(&(), ModelRequest::new(Vec::new()).with_model(marker))
        .await;

    let sentinel = "egress-per-request-sentinel";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    assert!(descriptor.is_external, "{descriptor:?}");
                    break;
                }
                assert_ne!(
                    descriptor.service, sentinel,
                    "a per-request cloud tag was not recorded as egress"
                );
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
}
