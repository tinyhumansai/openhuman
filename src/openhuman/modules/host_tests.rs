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
