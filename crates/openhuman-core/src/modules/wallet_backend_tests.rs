use super::*;
use serde_json::json;

#[tokio::test]
async fn with_no_handler_the_call_answers_the_backend_unavailable_sentinel() {
    let error = call_wallet_backend(Config::default(), WalletBackendCall::Routes)
        .await
        .expect_err("nothing is registered");
    assert!(
        error.starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "got: {error}"
    );
}

#[tokio::test]
async fn a_registered_handler_answers_the_call() {
    register_wallet_backend("test-wallet-backend", |_config, call| {
        Box::pin(async move {
            match call {
                WalletBackendCall::SwapTransaction(body) => Ok(json!({ "echo": body })),
                other => Err(format!("unexpected call: {other:?}")),
            }
        })
    });
    let data = call_wallet_backend(
        Config::default(),
        WalletBackendCall::SwapTransaction(json!({ "chainId": 1 })),
    )
    .await
    .expect("handler answers");
    assert_eq!(data, json!({ "echo": { "chainId": 1 } }));
    assert!(unregister_wallet_backend("test-wallet-backend"));
}

#[test]
fn a_module_request_converts_to_the_matching_call() {
    use tinywallet_bus::wire::WalletBackendRequest as Request;
    assert_eq!(
        WalletBackendCall::from(Request::Routes),
        WalletBackendCall::Routes
    );
    assert_eq!(
        WalletBackendCall::from(Request::SwapTransaction {
            body: json!({ "a": 1 })
        }),
        WalletBackendCall::SwapTransaction(json!({ "a": 1 }))
    );
    assert_eq!(
        WalletBackendCall::from(Request::BridgeTransaction {
            body: json!({ "b": 2 })
        }),
        WalletBackendCall::BridgeTransaction(json!({ "b": 2 }))
    );
}
