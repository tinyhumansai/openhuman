use super::*;

#[tokio::test]
async fn a_disabled_client_is_unavailable_before_core_startup() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let result = ModuleClient::new(config)
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[tokio::test]
async fn unknown_modules_are_not_loaded_or_reported_using_untrusted_ids() {
    let client = ModuleClient::new(Config::default());
    let result = client
        .call::<serde_json::Value>("private-user-path-and-token", "Anything", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
    assert!(!result
        .unwrap_err()
        .to_string()
        .contains("private-user-path-and-token"));
}

#[cfg(not(feature = "modules"))]
#[tokio::test]
async fn a_client_without_the_loader_never_falls_back_to_an_implementation() {
    let result = ModuleClient::new(Config::default())
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[cfg(feature = "modules")]
#[test]
fn remote_fault_text_is_not_part_of_the_public_error() {
    let error = tinybus::Error::MethodFailed {
        name: "private-provider".to_string(),
        message: "secret-token /home/private-user document content".to_string(),
    };
    assert_eq!(classify(&error).0, ModuleCallError::ModuleFault);
    assert_eq!(
        classify(&error).0.to_string(),
        "MODULE_CALL_REPORTED: module execution failed"
    );
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

#[tokio::test]
async fn confidential_calls_do_not_fall_back_when_disabled() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let result = ModuleClient::new(config)
        .call_confidential::<serde_json::Value>("tinywallet", "SignTransaction", ("secret-value",))
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[cfg(feature = "modules")]
#[test]
fn wire_unknown_method_is_an_incompatible_contract() {
    let error = tinybus::Error::MethodFailed {
        name: tinybus::Error::UNKNOWN_METHOD.to_string(),
        message: "private-response-content".to_string(),
    };
    assert_eq!(classify(&error).0, ModuleCallError::IncompatibleContract);
    assert_eq!(
        classify(&tinybus::Error::ConnectionClosed).0,
        ModuleCallError::TransportFailed
    );
}

#[cfg(feature = "modules")]
#[derive(Clone)]
struct BusFixture(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(feature = "modules")]
#[tinybus::interface(name = "test.ModuleClient")]
impl BusFixture {
    async fn sum(&self, left: u32, right: u32) -> tinybus::Result<u32> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
}

#[cfg(all(feature = "modules", feature = "crash-reporting"))]
#[test]
fn independent_invocation_faults_report_separately_around_a_success() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let proxy = runtime.block_on(async {
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
                ObjectPath::new("/test/ModuleClient").unwrap(),
                BusFixture(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))),
            )
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
    });
    let (proxy, _service) = proxy;
    let record = registry::find("tinysearch").unwrap();
    let events = sentry::test::with_captured_events(|| {
        runtime.block_on(async {
            assert_eq!(
                invoke_proxy::<()>(record, &proxy, "Fault", (), false).await,
                Err(ModuleCallError::ModuleFault)
            );
            assert_eq!(
                invoke_proxy::<u32>(record, &proxy, "Sum", (20, 22), false).await,
                Ok(42)
            );
            assert_eq!(
                invoke_proxy::<()>(record, &proxy, "Fault", (), false).await,
                Err(ModuleCallError::ModuleFault)
            );
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

#[cfg(all(feature = "modules", feature = "crash-reporting"))]
#[test]
fn cached_native_module_unavailability_is_reported_once() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let proxy = runtime.block_on(async {
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
                ObjectPath::new("/test/ModuleClient").unwrap(),
                BusFixture(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))),
            )
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
    });
    let (proxy, _service) = proxy;
    let record = registry::find("tinybox").unwrap();
    let events = sentry::test::with_captured_events(|| {
        runtime.block_on(async {
            for _ in 0..2 {
                assert_eq!(
                    invoke_proxy::<()>(record, &proxy, "Unavailable", (), false).await,
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

#[cfg(feature = "modules")]
#[tokio::test]
async fn bus_calls_preserve_tuple_arity_and_never_downgrade_confidentiality() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    service
        .serve_at(
            ObjectPath::new("/test/ModuleClient").unwrap(),
            BusFixture(calls.clone()),
        )
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
    let record = registry::find("tinysearch").unwrap();
    assert_eq!(
        invoke_proxy::<u32>(record, &proxy, "Sum", (20_u32, 22_u32), false).await,
        Ok(42)
    );
    assert_eq!(
        invoke_proxy::<u32>(record, &proxy, "Sum", (20_u32, 22_u32), true).await,
        Err(ModuleCallError::TransportFailed)
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        invoke_proxy::<()>(record, &proxy, "Fault", (), false).await,
        Err(ModuleCallError::ModuleFault)
    );
    assert_eq!(
        invoke_proxy::<()>(record, &proxy, "MissingMember", (), false).await,
        Err(ModuleCallError::IncompatibleContract)
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn unknown_module_failures_emit_only_safe_registry_metadata() {
    let events = sentry::test::with_captured_events(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = ModuleClient::new(Config::default());
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
