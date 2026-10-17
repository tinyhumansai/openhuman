use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use serde_json::json;
use tinyflows_bus::schedule::{DOCUMENT_INTERFACE, DOCUMENT_METHOD};

async fn port() -> (tinybus::Proxy, CallbackAddress) {
    let config = Config::default();
    // Start the module runtime and attach the module before touching the bus.
    module::proxy(&config).await.expect("module attaches");
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let scope = DocScope::on_backend(&config, backend, Scope::local());
    let address = address(&scope).await.expect("port is served");
    let runtime = crate::modules::runtime::runtime().await.unwrap();
    let proxy = runtime
        .connection()
        .proxy(
            &address.destination,
            &address.object_path,
            DOCUMENT_INTERFACE,
        )
        .expect("the contract's document interface is the one served");
    (proxy, address)
}

async fn ask(proxy: &tinybus::Proxy, request: Value) -> Value {
    proxy.call(DOCUMENT_METHOD, (request,)).await.unwrap()
}

#[tokio::test]
async fn the_port_serves_the_contracts_document_interface_and_reports_capabilities() {
    let (proxy, _) = port().await;
    let reply = ask(&proxy, json!({"op": "capabilities"})).await;
    assert!(reply["value"]["transactions"].is_boolean(), "{reply}");
}

#[tokio::test]
async fn a_second_absent_write_comes_back_as_a_conflict_kind() {
    let (proxy, _) = port().await;
    let absent = serde_json::to_value(tinystoragedrivers::Precondition::Absent).unwrap();
    let put =
        json!({"op":"put","collection":"cron.jobs","id":"dup","doc":{"a":1},"precondition":absent});
    // The collection must exist first; the module declares it, the port relays.
    let ensured = ask(&proxy, json!({"op":"ensure","spec":{"name":"cron.jobs"}})).await;
    assert!(ensured.get("error").is_none(), "{ensured}");
    let first = ask(&proxy, put.clone()).await;
    assert!(first.get("error").is_none(), "{first}");
    let second = ask(&proxy, put).await;
    assert_eq!(second["error"]["kind"], "conflict", "{second}");
}

#[tokio::test]
async fn a_collection_outside_the_schedule_port_is_refused() {
    let (proxy, _) = port().await;
    let none = serde_json::to_value(tinystoragedrivers::Precondition::None).unwrap();
    for request in [
        json!({"op":"get","collection":"memory_items","id":"x"}),
        json!({"op":"count","collection":"flows","filter":{}}),
        json!({"op":"ensure","spec":{"name":"secrets"}}),
        json!({"op":"atomic_batch","ops":[
            {"op":"put","collection":"cron.jobs","id":"ok","doc":{},"precondition":none},
            {"op":"delete","collection":"approvals","id":"x","precondition":none}
        ]}),
    ] {
        let reply = ask(&proxy, request.clone()).await;
        assert_eq!(
            reply["error"]["kind"], "invalid_input",
            "{request} -> {reply}"
        );
    }
}

#[tokio::test]
async fn an_unknown_operation_is_refused_not_guessed() {
    let (proxy, _) = port().await;
    let reply = ask(
        &proxy,
        json!({"op":"drop_collection","collection":"cron.jobs"}),
    )
    .await;
    assert_eq!(reply["error"]["kind"], "invalid_input", "{reply}");
}

#[tokio::test]
async fn one_object_is_served_per_scope_and_reused() {
    let config = Config::default();
    module::proxy(&config).await.unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let a = DocScope::on_backend(&config, Arc::clone(&backend), Scope::local());
    let b = DocScope::on_backend(&config, backend, crate::storage::scope_for_agent("other"));
    let (first, again, other) = (
        address(&a).await.unwrap(),
        address(&a).await.unwrap(),
        address(&b).await.unwrap(),
    );
    assert_eq!(first.object_path, again.object_path);
    assert_ne!(first.object_path, other.object_path);
    assert_eq!(first.destination, module::HOST_BUS_NAME);
}
