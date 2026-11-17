use super::*;
use tinybus::{broker::Broker, transport::memory::MemoryBus, ObjectPath};

const OWNER: &str = "ai.tinyhumans.test.Module";
const HOST: &str = "ai.tinyhumans.test.Host";
const PATH: &str = "/ai/tinyhumans/test/Host";

struct Echo;

#[tinybus::interface(name = "ai.tinyhumans.test.Host")]
impl Echo {
    async fn echo(&self, text: String) -> tinybus::Result<String> {
        Ok(text)
    }
}

async fn host_bus() -> (
    MemoryBus,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
) {
    let transport = MemoryBus::new();
    let task = Broker::new().spawn(transport.clone());
    let host = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    host.serve_at(
        ObjectPath::new(PATH).unwrap(),
        OwnedBy::new(Echo, &host, OWNER),
    )
    .await
    .unwrap();
    host.request_name(HOST).await.unwrap();
    (transport, host, task)
}

async fn echo(caller: &Connection) -> tinybus::Result<String> {
    caller
        .proxy(HOST, PATH, HOST)
        .unwrap()
        .call("Echo", ("hello".to_owned(),))
        .await
}

#[tokio::test]
async fn a_peer_that_is_not_the_module_is_refused() {
    let (transport, _host, _broker) = host_bus().await;
    let stranger = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let refused = echo(&stranger).await.unwrap_err();
    assert!(format!("{refused:?}").contains(REFUSAL), "{refused:?}");
}

#[tokio::test]
async fn the_peer_that_owns_the_module_name_is_answered() {
    let (transport, _host, _broker) = host_bus().await;
    let module = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    module.request_name(OWNER).await.unwrap();
    assert_eq!(echo(&module).await.unwrap(), "hello");
}

#[tokio::test]
async fn with_no_module_loaded_every_caller_is_refused() {
    let (_transport, host, _broker) = host_bus().await;
    let refused = echo(&host).await.unwrap_err();
    assert!(format!("{refused:?}").contains(REFUSAL), "{refused:?}");
}

#[tokio::test]
async fn a_served_callback_does_not_keep_its_connection_alive() {
    let (_transport, host, _broker) = host_bus().await;
    let weak = host.downgrade();
    drop(host);
    // The connection serves `OwnedBy`, so a strong handle inside it would be
    // a cycle: neither would ever be freed.
    assert!(
        weak.upgrade().is_none(),
        "the served callback still owns the connection that serves it"
    );
}
