//! Composition tests for [`super::client`]: the tinybus client, the registry and
//! the shared Sentry reporter wired together. The client's own behaviour (tuple
//! arity, confidentiality, error classification, object clients, deadlines) is
//! tested in `tinybus::module`.

use super::*;
use tinybus::module::ModuleCallError;

#[tokio::test]
async fn a_disabled_client_is_unavailable_before_core_startup() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let result = client(config)
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[tokio::test]
async fn confidential_calls_do_not_fall_back_when_disabled() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let result = client(config)
        .call_confidential::<serde_json::Value>("tinywallet", "SignTransaction", ("secret-value",))
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[tokio::test]
async fn unknown_modules_are_not_loaded_or_reported_using_untrusted_ids() {
    let result = client(Config::default())
        .call::<serde_json::Value>("private-user-path-and-token", "Anything", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
    assert!(!result
        .unwrap_err()
        .to_string()
        .contains("private-user-path-and-token"));
}

#[test]
fn subsequent_product_reports_are_demoted() {
    for error in [
        ModuleCallError::Unavailable,
        ModuleCallError::IncompatibleContract,
        ModuleCallError::TransportFailed,
        ModuleCallError::ModuleFault,
    ] {
        assert!(crate::core::observability::is_module_unavailable_message(
            &error.to_string()
        ));
    }
}

#[cfg(feature = "crash-reporting")]
mod reported {
    use super::*;
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};

    #[derive(Clone)]
    struct BusFixture;

    #[tinybus::interface(name = "test.ModuleClient")]
    impl BusFixture {
        async fn sum(&self, left: u32, right: u32) -> tinybus::Result<u32> {
            Ok(left + right)
        }

        async fn fault(&self) -> tinybus::Result<()> {
            Err(tinybus::Error::MethodFailed {
                name: "test.ProviderFault".to_string(),
                message: "secret-value /home/private-user/payload".to_string(),
            })
        }

        async fn unavailable(&self) -> tinybus::Result<()> {
            Err(tinybus::Error::ModuleUnavailable {
                module: "test.ModuleClient".to_string(),
                state: "faulted".to_string(),
                detail: "module stopped accepting calls".to_string(),
            })
        }

        async fn malformed_json(&self) -> tinybus::Result<String> {
            Ok("{ private response data".to_owned())
        }
    }

    /// A proxy to the fixture object, and the service that must outlive it.
    async fn fixture() -> (tinybus::Proxy, Connection) {
        let bus = MemoryBus::new();
        Broker::new().spawn(bus.clone());
        let service = Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap();
        let caller = Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap();
        service
            .serve_at(ObjectPath::new("/test/ModuleClient").unwrap(), BusFixture)
            .await
            .unwrap();
        service.request_name("test.ModuleClient").await.unwrap();
        let proxy = caller
            .proxy(
                "test.ModuleClient",
                "/test/ModuleClient",
                "test.ModuleClient",
            )
            .unwrap();
        (proxy, service)
    }

    fn current_thread() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn independent_invocation_faults_report_separately_around_a_success() {
        let runtime = current_thread();
        let (proxy, _service) = runtime.block_on(fixture());
        let client = fixture_client(proxy);
        let events = sentry::test::with_captured_events(|| {
            runtime.block_on(async {
                let fault = || client.call::<()>("tinysearch", "Fault", ());
                assert_eq!(fault().await, Err(ModuleCallError::ModuleFault));
                assert_eq!(
                    client.call::<u32>("tinysearch", "Sum", (20, 22)).await,
                    Ok(42)
                );
                assert_eq!(fault().await, Err(ModuleCallError::ModuleFault));
            });
        });
        assert_eq!(events.len(), 2, "{events:?}");
        for event in &events {
            assert_eq!(
                event.tags.get("reason_code").map(String::as_str),
                Some("module_fault")
            );
            let captured = format!("{event:?}");
            assert!(!captured.contains("secret-value"));
            assert!(!captured.contains("/home/private-user"));
        }
    }

    #[test]
    fn cached_native_module_unavailability_is_reported_once() {
        let runtime = current_thread();
        let (proxy, _service) = runtime.block_on(fixture());
        let client = fixture_client(proxy);
        let events = sentry::test::with_captured_events(|| {
            runtime.block_on(async {
                for _ in 0..2 {
                    assert_eq!(
                        client.call::<()>("tinybox", "Unavailable", ()).await,
                        Err(ModuleCallError::Unavailable)
                    );
                }
            });
        });
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0].tags.get("reason_code").map(String::as_str),
            Some("module_unavailable")
        );
    }

    #[test]
    fn malformed_inner_json_from_a_real_bus_reply_is_reported_as_a_sanitized_fault() {
        let runtime = current_thread();
        let (proxy, _service) = runtime.block_on(fixture());
        let client = fixture_client(proxy);
        let events = sentry::test::with_captured_events(|| {
            assert_eq!(
                runtime.block_on(client.call_json::<serde_json::Value>(
                    "tinyconnectors",
                    "MalformedJson",
                    ()
                )),
                Err(ModuleCallError::ModuleFault)
            );
        });
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0].tags.get("reason_code").map(String::as_str),
            Some("module_fault")
        );
        assert!(!format!("{events:?}").contains("private response data"));
    }

    #[test]
    fn unknown_module_failures_emit_only_safe_registry_metadata() {
        let events = sentry::test::with_captured_events(|| {
            current_thread().block_on(async {
                let client = client(Config::default());
                for _ in 0..2 {
                    let error = client
                        .call::<serde_json::Value>(
                            "private-token /home/private-user",
                            "secret-member",
                            ("private-content",),
                        )
                        .await
                        .unwrap_err();
                    assert_eq!(error, ModuleCallError::Unavailable);
                    assert!(!error.to_string().contains("private"));
                }
            });
        });
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0].tags.get("module").map(String::as_str),
            Some("unregistered")
        );
        assert_eq!(
            events[0].tags.get("reason_code").map(String::as_str),
            Some("unknown_module")
        );
        assert!(!format!("{events:?}").contains("private"));
    }
}
