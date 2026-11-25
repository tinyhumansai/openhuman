//! Real TinySearch module admission and wire contract, isolated from other runtimes.
use tinysearch_bus::SearchConfig;

#[tokio::test]
async fn pinned_search_module_is_admitted_with_the_registered_release() {
    use tinybus::{broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
    let file = openhuman_core::modules::local_artifact("tinysearch").expect(
        "build tinysearch with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR",
    );
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let config = SearchConfig {
        enabled: false,
        ..Default::default()
    };
    let info = host
        .load_file_with_config(file, serde_json::to_value(config).unwrap())
        .expect("published artifact is admitted by the host loader");
    assert_eq!(info.name, "tinysearch");
    // The registry pin is the single statement of which release is shipped.
    let pinned = openhuman_core::modules::registry::find("tinysearch").expect("registered");
    assert_eq!(info.version, pinned.version);
    host.shutdown(std::time::Duration::from_secs(5)).await;
    broker_task.abort();
}
