use super::*;

#[tokio::test]
async fn configured_calls_keep_the_lock_through_invocation() {
    use std::sync::{Arc, Mutex};
    let active = Arc::new(Mutex::new(String::new()));
    let (a_entered_tx, a_entered_rx) = tokio::sync::oneshot::channel();
    let (release_a_tx, release_a_rx) = tokio::sync::oneshot::channel();
    let a_active = active.clone();
    let a = tokio::spawn(async move {
        with_module_lock(|| async move {
            *a_active.lock().unwrap() = "A".into();
            a_entered_tx.send(()).unwrap();
            release_a_rx.await.unwrap();
            assert_eq!(*a_active.lock().unwrap(), "A");
            Ok::<_, String>(())
        })
        .await
        .unwrap();
    });
    a_entered_rx.await.unwrap();
    let b_active = active.clone();
    let (b_started_tx, b_started_rx) = tokio::sync::oneshot::channel();
    let b = tokio::spawn(async move {
        b_started_tx.send(()).unwrap();
        with_module_lock(|| async move {
            *b_active.lock().unwrap() = "B".into();
            Ok::<_, String>(())
        })
        .await
        .unwrap();
    });
    b_started_rx.await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(*active.lock().unwrap(), "A");
    release_a_tx.send(()).unwrap();
    a.await.unwrap();
    b.await.unwrap();
    assert_eq!(*active.lock().unwrap(), "B");
}

/// A search module whose `ExecuteTool` takes `delay` to answer, like a
/// provider synthesizing a `web_answer_tool` reply.
struct SlowSearch {
    delay: std::time::Duration,
}

#[async_trait::async_trait]
impl tinybus::service::Interface for SlowSearch {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new(names::INTERFACE).expect("contract interface")
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new(names::methods::EXECUTE_TOOL).expect("contract member")]
    }

    async fn call(
        &self,
        _member: &tinybus::MemberName,
        _args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        tokio::time::sleep(self.delay).await;
        Ok(serde_json::json!({ "answered": true }))
    }
}

/// The service connection is returned too: dropping it would unserve the mock.
async fn slow_search_proxy(delay: std::time::Duration) -> (tinybus::Connection, tinybus::Proxy) {
    use tinybus::transport::memory::MemoryBus;
    let bus = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let service = tinybus::Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service");
    service
        .serve_at(
            tinybus::ObjectPath::new(names::OBJECT_PATH).expect("contract path"),
            SlowSearch { delay },
        )
        .await
        .expect("serve");
    service.request_name(names::INTERFACE).await.expect("name");
    let client = tinybus::Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client");
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .expect("proxy");
    (service, proxy)
}

fn web_answer_request() -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: "web_answer_tool".into(),
        arguments: serde_json::json!({ "query": "q" }),
    }
}

#[tokio::test(start_paused = true)]
async fn execute_tool_outlives_the_bus_default_timeout() {
    // Production: `web_answer_tool` / `web_search_tool` failed at the 30 s bus
    // default while the module was still working.
    let delay = tinybus::connection::DEFAULT_TIMEOUT + std::time::Duration::from_secs(15);
    assert!(delay < EXECUTE_TOOL_TIMEOUT);

    // Control: the bus default really does cut this call off.
    let (_service, proxy) = slow_search_proxy(delay).await;
    let cut_off = proxy
        .clone()
        .call::<serde_json::Value>(names::methods::EXECUTE_TOOL, (web_answer_request(),))
        .await;
    assert!(
        cut_off.is_err(),
        "the default deadline must fire: {cut_off:?}"
    );

    let answered: serde_json::Value = call_execute_tool(proxy, web_answer_request())
        .await
        .expect("ExecuteTool runs under its own longer deadline");
    assert_eq!(answered["answered"], true);
}

#[test]
fn the_execute_tool_deadline_stays_under_the_harness_tool_deadline() {
    assert!(EXECUTE_TOOL_TIMEOUT > tinybus::connection::DEFAULT_TIMEOUT);
    assert!(
        EXECUTE_TOOL_TIMEOUT.as_secs() < crate::tools::timeout::DEFAULT_TIMEOUT_SECS,
        "the harness would kill the call before the module could answer"
    );
}

#[test]
fn the_execute_tool_deadline_follows_a_raised_tool_budget() {
    // A user who raises the action timeout to 10 minutes must not have the
    // search call cut off at the fixed 90 s floor.
    assert_eq!(
        execute_tool_timeout_for(600),
        std::time::Duration::from_secs(570)
    );
    assert!(execute_tool_timeout_for(600).as_secs() < 600);
    // The default budget keeps today's 90 s, and a tighter budget never drops
    // the bus deadline below the floor (the harness deadline is tighter).
    assert_eq!(
        execute_tool_timeout_for(crate::tools::timeout::DEFAULT_TIMEOUT_SECS),
        EXECUTE_TOOL_TIMEOUT
    );
    assert_eq!(execute_tool_timeout_for(10), EXECUTE_TOOL_TIMEOUT);
}

#[tokio::test]
#[ignore = "requires TINYSEARCH_TEST_MODULE pointing to the byte-verified release artifact"]
async fn released_search_artifact_serves_host_contract_and_rejects_disabled_execution() {
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
    assert_eq!(info.version, "0.4.1");
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
    let failure = call_execute_tool::<ExecuteToolResponse>(proxy, web_answer_request())
        .await
        .expect_err("disabled module refuses execution without contacting providers");
    assert!(
        failure.to_string().contains("search is disabled"),
        "{failure}"
    );
    host.shutdown(std::time::Duration::from_secs(5)).await;
    broker_task.abort();
}
