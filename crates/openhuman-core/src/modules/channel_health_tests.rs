use super::*;

#[cfg(feature = "modules")]
#[tokio::test]
async fn provider_credentials_never_reach_an_unattested_bus() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    #[derive(Clone)]
    struct Unattested(Arc<AtomicUsize>);
    #[tinybus::interface(name = "test.ChannelHealth")]
    impl Unattested {
        async fn check_channel_health(
            &self,
            _: ChannelsConfig,
            _: HttpProxyPolicy,
            _: u64,
        ) -> tinybus::Result<Vec<ChannelHealthReport>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
    }
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    service
        .serve_at(
            ObjectPath::new("/test/ChannelHealth").unwrap(),
            Unattested(calls.clone()),
        )
        .await
        .unwrap();
    service.request_name("test.ChannelHealth").await.unwrap();
    let proxy = caller
        .proxy(
            "test.ChannelHealth",
            "/test/ChannelHealth",
            "test.ChannelHealth",
        )
        .unwrap();
    let error = check(
        &crate::modules::ops::fixture_client(proxy),
        ChannelsConfig::default(),
        HttpProxyPolicy::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error, ModuleCallError::TransportFailed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
