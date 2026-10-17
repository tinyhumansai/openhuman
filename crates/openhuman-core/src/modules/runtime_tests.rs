use super::{is_started, runtime};

/// The process-global runtime is stable within one caller runtime.
#[tokio::test]
async fn the_module_bus_is_a_singleton_and_serves_proxies() {
    let first = runtime().await.expect("runtime should start");
    assert!(is_started());
    let second = runtime().await.expect("runtime should be reused");
    assert!(
        std::ptr::eq(first, second),
        "runtime() handed out two different runtimes"
    );

    // Building a proxy is a local operation — it validates names and nothing
    // else. Nothing has claimed the name, so the call fails rather than
    // hanging, which is what makes `ensure_loaded` worth having.
    let proxy = first
        .proxy(
            "ai.tinyhumans.tinydocs.Documents",
            "/ai/tinyhumans/tinydocs/Documents",
        )
        .expect("registry names should be well formed");
    let result: tinybus::Result<serde_json::Value> = proxy.call("GenerateDocx", ()).await;
    assert!(result.is_err(), "an unloaded module should not answer");

    // A name that cannot be a bus name is refused without reaching the bus.
    assert!(first.proxy("not a bus name", "/nope").is_err());
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn wallet_host_callback_is_served_before_the_module_is_loaded() {
    let workspace = tempfile::tempdir().expect("isolated wallet workspace");
    let _workspace_env =
        crate::web3::wallet::test_support::set_workspace_env_for_test(&workspace).await;
    let runtime = runtime().await.expect("runtime should start");
    let proxy = runtime
        .proxy(
            tinywallet_bus::HOST_INTERFACE,
            tinywallet_bus::HOST_OBJECT_PATH,
        )
        .expect("wallet callback contract names are valid");

    let response: tinywallet_bus::wire::HostResponse = proxy
        .call(tinywallet_bus::names::host_methods::WALLET_ACCOUNTS, ())
        .await
        .expect("the host must serve callbacks before resolving the wallet module");
    assert!(matches!(
        response,
        tinywallet_bus::wire::HostResponse::WalletAccounts(_)
    ));
}
