use super::*;

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

#[cfg(feature = "flows")]
#[tokio::test]
async fn registry_id_matches_the_live_modules_bus_address_not_package_name() {
    let config = crate::config::Config::default();
    let proxy = crate::modules::tinyflows::copilot_proxy(&config)
        .await
        .expect("linked TinyFlows module should be admitted and serving");
    let _: Vec<tinyflows_bus::copilot::ResourceFile> = proxy
        .call("ListResources", ())
        .await
        .expect("the resource call waits for module initialization");

    let record = crate::modules::registry::find(crate::modules::tinyflows::MODULE_ID)
        .expect("TinyFlows registry record");
    let runtime = runtime().await.expect("module runtime is already serving");
    let loaded = runtime
        .host()
        .list()
        .into_iter()
        .find(|module| {
            module.manifest.bus_name.as_str() == record.bus_name
                && module.manifest.object_path.as_str() == record.object_path
        })
        .expect("the registry bus address identifies the loaded module");

    assert_eq!(record.id, "tinyflows");
    assert_eq!(loaded.name, "tinyflows-module");
    assert_ne!(loaded.name, record.id);
    assert!(
        is_module_serving(record.id),
        "the registry id must report the enabled, live module despite its package name; info={loaded:#?}"
    );
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

    assert!(
        proxy
            .is_available()
            .await
            .expect("ask the bus for the owner"),
        "the host must serve callbacks before resolving the wallet module"
    );
    // Only TinyWallet may call them (TS-E2E-012); the host's own connection is
    // not TinyWallet.
    let refused: tinybus::Result<tinywallet_bus::wire::HostResponse> = proxy
        .call(tinywallet_bus::names::host_methods::WALLET_ACCOUNTS, ())
        .await;
    // The ownership refusal itself, not any error (a missing callback errs too).
    assert!(
        refused.as_ref().is_err_and(
            |error| format!("{error:?}").contains(crate::modules::callback_owner::REFUSAL)
        ),
        "a caller that is not TinyWallet was not refused by the owner check: {refused:?}"
    );
}
