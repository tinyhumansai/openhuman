//! Real TinySearch module admission and wire contract, isolated from other runtimes.
use tinysearch_bus::{names, ExecuteToolRequest, ExecuteToolResponse, ListToolsResponse};

#[tokio::test]
async fn pinned_search_module_serves_host_contract_and_rejects_disabled_execution() {
    use tinybus::{broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
    let file = std::env::var_os("TINYSEARCH_TEST_MODULE").expect("released artifact path");
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let config = tinysearch_bus::SearchConfig {
        enabled: false,
        ..Default::default()
    };
    let info = host
        .load_file_with_config(
            std::path::PathBuf::from(file),
            serde_json::to_value(config).unwrap(),
        )
        .expect("published artifact is admitted by the host loader");
    assert_eq!(info.name, "tinysearch");
    assert_eq!(info.version, "0.4.2");
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
