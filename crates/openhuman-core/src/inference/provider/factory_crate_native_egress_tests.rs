use super::*;

#[tokio::test]
async fn native_claude_turn_routes_disclose_pinned_models() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    let _storage = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let configured_sdk = "egress-sdk-configured-marker";
    let pinned_sdk = "egress-sdk-pinned-marker";
    let mut sdk_config = Config::default();
    sdk_config.chat_provider = Some(format!("claude_agent_sdk:{configured_sdk}"));
    crate::inference::turn_models::TurnModelSource::new_crate_native(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        Arc::new(sdk_config),
    )
    .build(pinned_sdk, 0.0, None, None)
    .await
    .expect("Claude Agent SDK turn set should build");

    let configured_code = "egress-code-configured-marker";
    let pinned_code = "egress-code-pinned-marker";
    let mut code_config = Config::default();
    code_config.chat_provider = Some(format!("claude-code:{configured_code}"));
    // Egress is disclosed once the effective model is selected, before the
    // environment probe. The test therefore remains valid on hosts without the
    // Claude Code CLI.
    let _ = crate::inference::turn_models::TurnModelSource::new_crate_native(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        Arc::new(code_config),
    )
    .build(pinned_code, 0.0, None, None)
    .await;

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
