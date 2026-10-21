use super::*;
use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection};

#[tokio::test]
async fn ready_only_child_call_does_not_pin_or_load_an_unopened_module() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let caller = Connection::connect(bus.connect().await.expect("connect memory bus"))
        .await
        .expect("create memory-bus client");
    let root = crate::modules::ops::fixture_client(
        caller
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
            .expect("create inert test proxy"),
    );
    let port = TinyMcpPort::new(root);

    let result = port
        .call_object_if_ready("/test/child", names::methods::STATUS, Value::Null, false)
        .await;

    assert_eq!(result, Err(ModuleCallError::Unavailable));
}
