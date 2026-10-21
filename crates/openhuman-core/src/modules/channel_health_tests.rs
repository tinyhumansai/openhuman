use super::*;
use tinychannels_bus::ChannelHealthState;

#[cfg(feature = "modules")]
type ObservedHealth = (bool, Vec<String>, u64);

#[cfg(feature = "modules")]
#[derive(Clone)]
struct HealthResponseState {
    delay: std::time::Duration,
    fail: bool,
    observed: std::sync::Arc<std::sync::Mutex<Option<ObservedHealth>>>,
}

#[cfg(feature = "modules")]
#[derive(Clone)]
struct HealthResponder(HealthResponseState);

#[cfg(feature = "modules")]
#[async_trait::async_trait]
impl tinybus::service::Interface for HealthResponder {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new("test.ChannelHealth").unwrap()
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new("CheckChannelHealth").unwrap()]
    }

    async fn call(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        if member.as_str() != "CheckChannelHealth" {
            return Err(tinybus::Error::failed("unknown fixture member"));
        }
        let selectors = args[1]["service_selectors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect();
        *self.0.observed.lock().unwrap() = Some((
            args[0]["telegram"].is_object(),
            selectors,
            args[2].as_u64().unwrap_or_default(),
        ));
        tokio::time::sleep(self.0.delay).await;
        if self.0.fail {
            return Err(tinybus::Error::failed("fixture secret must be discarded"));
        }
        serde_json::to_value(vec![ChannelHealthReport {
            channel: "telegram".into(),
            state: ChannelHealthState::Healthy,
        }])
        .map_err(|_| tinybus::Error::failed("fixture response encoding failed"))
    }
}

#[cfg(feature = "modules")]
async fn health_proxy(state: HealthResponseState) -> tinybus::Proxy {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    service
        .serve_at(
            ObjectPath::new("/test/ChannelHealth").unwrap(),
            HealthResponder(state),
        )
        .await
        .unwrap();
    service.request_name("test.ChannelHealth").await.unwrap();
    caller
        .proxy(
            "test.ChannelHealth",
            "/test/ChannelHealth",
            "test.ChannelHealth",
        )
        .unwrap()
}

#[cfg(feature = "modules")]
fn health_inputs() -> (ChannelsConfig, HttpProxyPolicy) {
    (
        ChannelsConfig {
            telegram: Some(tinychannels_bus::config::TelegramConfig {
                bot_token: "selected-b-test-token".into(),
                chat_id: None,
                allowed_users: Vec::new(),
                stream_mode: Default::default(),
                draft_update_interval_ms: 1_000,
                silent_streaming: true,
                mention_only: false,
            }),
            ..Default::default()
        },
        HttpProxyPolicy {
            service_selectors: vec!["channel.telegram".into()],
            ..Default::default()
        },
    )
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn existing_health_adapter_passes_only_the_selected_provider_and_policy() {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let proxy = health_proxy(HealthResponseState {
        delay: std::time::Duration::ZERO,
        fail: false,
        observed: observed.clone(),
    })
    .await;
    let (channels, policy) = health_inputs();

    let reports = call_existing_proxy(
        &proxy,
        channels,
        policy,
        std::time::Duration::from_millis(100),
    )
    .await
    .expect("existing read-only member returns its health report");

    assert_eq!(reports[0].channel, "telegram");
    assert_eq!(reports[0].state, ChannelHealthState::Healthy);
    assert_eq!(
        observed.lock().unwrap().as_ref().unwrap(),
        &(true, vec!["channel.telegram".into()], 100)
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn existing_health_adapter_sanitizes_failure_and_bounds_timeout() {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let proxy = health_proxy(HealthResponseState {
        delay: std::time::Duration::from_millis(100),
        fail: false,
        observed: observed.clone(),
    })
    .await;
    let (channels, policy) = health_inputs();
    let failure = call_existing_proxy(
        &proxy,
        channels,
        policy,
        std::time::Duration::from_millis(10),
    )
    .await
    .expect_err("late provider must time out");
    assert_eq!(failure, ExistingHealthError::Failed);

    let failing_proxy = health_proxy(HealthResponseState {
        delay: std::time::Duration::ZERO,
        fail: true,
        observed: std::sync::Arc::new(std::sync::Mutex::new(None)),
    })
    .await;
    let (channels, policy) = health_inputs();
    let failure = call_existing_proxy(
        &failing_proxy,
        channels,
        policy,
        std::time::Duration::from_millis(100),
    )
    .await
    .expect_err("module error is reduced to a safe failure class");
    assert_eq!(failure, ExistingHealthError::Failed);
    assert_ne!(format!("{failure:?}"), "fixture secret must be discarded");
}

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
