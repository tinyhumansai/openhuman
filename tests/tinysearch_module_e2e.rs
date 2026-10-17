//! Real TinySearch module admission and wire contract, isolated from other runtimes.
use tinysearch_bus::{names, ExecuteToolRequest, ExecuteToolResponse, ListToolsResponse};

#[tokio::test]
async fn pinned_search_module_serves_host_contract_and_rejects_disabled_execution() {
    use tinybus::{broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
    let file = openhuman_core::modules::local_artifact("tinysearch").expect(
        "build tinysearch with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR",
    );
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let config = tinysearch_bus::SearchConfig {
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
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !client
            .list_names()
            .await
            .unwrap()
            .iter()
            .any(|name| name.as_str() == names::INTERFACE)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("module claims its contract interface");
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .unwrap();
    let tools: ListToolsResponse = proxy.call(names::methods::LIST_TOOLS, ()).await.unwrap();
    assert!(tools.tools.is_empty());
    let failure = proxy
        .call::<ExecuteToolResponse>(
            names::methods::EXECUTE_TOOL,
            (ExecuteToolRequest {
                name: "web_answer_tool".into(),
                arguments: serde_json::json!({ "query": "q" }),
            },),
        )
        .await
        .expect_err("disabled module refuses execution without contacting providers");
    assert!(
        failure.to_string().contains("search is disabled"),
        "{failure}"
    );
    host.shutdown(std::time::Duration::from_secs(5)).await;
    broker_task.abort();
}
