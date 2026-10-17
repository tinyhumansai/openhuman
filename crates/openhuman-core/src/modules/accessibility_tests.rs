use super::*;

#[cfg(not(feature = "modules"))]
#[tokio::test]
async fn permissions_without_a_loader_are_explicitly_unavailable() {
    assert!(matches!(
        AccessibilityClient::new(&Config::default())
            .permissions()
            .await,
        Err(AccessibilityError::Bus(ModuleCallError::Unavailable))
    ));
}

#[cfg(feature = "modules")]
#[derive(Clone)]
struct Fixture(std::sync::Arc<std::sync::Mutex<(PermissionState, Vec<String>)>>);

#[cfg(feature = "modules")]
#[tinybus::interface(name = "test.Accessibility")]
impl Fixture {
    async fn accessibility_permissions(&self) -> tinybus::Result<DesktopResponse> {
        let mut state = self.0.lock().unwrap();
        state.1.push("permissions".into());
        Ok(DesktopResponse::ok(
            "accessibility-permissions",
            serde_json::to_value(PermissionStatus {
                accessibility: PermissionState::Unsupported,
                input_monitoring: PermissionState::Unsupported,
                microphone: state.0.clone(),
            })
            .unwrap(),
        ))
    }

    async fn accessibility_request_permission(
        &self,
        kind: PermissionKind,
    ) -> tinybus::Result<DesktopResponse> {
        assert_eq!(kind, PermissionKind::Microphone);
        {
            let mut state = self.0.lock().unwrap();
            state.1.push("request".into());
            state.0 = PermissionState::Granted;
        }
        self.accessibility_permissions().await
    }

    async fn accessibility_focus(&self, _: FocusQuery) -> tinybus::Result<DesktopResponse> {
        self.0.lock().unwrap().1.push("focus".into());
        Ok(DesktopResponse::ok(
            "accessibility-focus",
            serde_json::Value::Null,
        ))
    }

    async fn globe_read(&self, request: GlobeRead) -> tinybus::Result<DesktopResponse> {
        assert_eq!(request.handle.0, "opaque-lease");
        assert_eq!(request.acknowledged_batch, Some(4));
        Ok(DesktopResponse::ok(
            "globe-read",
            serde_json::to_value(GlobeBatch {
                handle: request.handle,
                batch: 5,
                status: GlobeHotkeyStatus {
                    supported: true,
                    running: true,
                    input_monitoring_permission: PermissionState::Granted,
                    last_error: None,
                    events_pending: 0,
                },
                events: vec![tinycomputer_bus::accessibility::GlobeEvent::Down],
                overflow: false,
            })
            .unwrap(),
        ))
    }
}

#[cfg(feature = "modules")]
async fn fixture(
    permission: PermissionState,
) -> (AccessibilityClient, Fixture, tinybus::Connection) {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let fixture = Fixture(std::sync::Arc::new(std::sync::Mutex::new((
        permission,
        vec![],
    ))));
    service
        .serve_at(
            ObjectPath::new("/test/Accessibility").unwrap(),
            fixture.clone(),
        )
        .await
        .unwrap();
    service.request_name("test.Accessibility").await.unwrap();
    let proxy = caller
        .proxy(
            "test.Accessibility",
            "/test/Accessibility",
            "test.Accessibility",
        )
        .unwrap();
    (
        AccessibilityClient {
            client: crate::modules::ops::fixture_client(proxy),
            computer_enabled: true,
        },
        fixture,
        service,
    )
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn permissions_use_the_zero_argument_bus_member() {
    let (client, fixture, _service) = fixture(PermissionState::Granted).await;
    assert_eq!(
        client.permissions().await.unwrap().microphone,
        PermissionState::Granted
    );
    assert_eq!(fixture.0.lock().unwrap().1, ["permissions"]);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn one_shot_capture_requests_unknown_microphone_permission() {
    let (client, fixture, _service) = fixture(PermissionState::Unknown).await;
    client.microphone_permission(true).await.unwrap();
    assert_eq!(
        fixture.0.lock().unwrap().1,
        ["permissions", "request", "permissions"]
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn continuous_capture_does_not_prompt_for_undetermined_permission() {
    let (client, fixture, _service) = fixture(PermissionState::Unknown).await;
    client.microphone_permission(false).await.unwrap();
    assert_eq!(fixture.0.lock().unwrap().1, ["permissions"]);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn denied_microphone_permission_fails_without_requesting_access() {
    let (client, fixture, _service) = fixture(PermissionState::Denied).await;
    assert!(matches!(client.microphone_permission(true).await,
        Err(AccessibilityError::Operation(error)) if error.code == "PERMISSION_DENIED"));
    assert_eq!(fixture.0.lock().unwrap().1, ["permissions"]);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn focus_never_retries_over_a_nonconfidential_transport() {
    let (client, fixture, _service) = fixture(PermissionState::Granted).await;
    assert!(matches!(
        client.focus(FocusQuery { verbose: true }).await,
        Err(AccessibilityError::Bus(ModuleCallError::TransportFailed))
    ));
    assert!(fixture.0.lock().unwrap().1.is_empty());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn reliable_globe_reads_forward_the_lease_and_acknowledged_batch() {
    let (client, _, _service) = fixture(PermissionState::Granted).await;
    let batch = client
        .globe_read(GlobeRead {
            handle: GlobeHandle("opaque-lease".into()),
            acknowledged_batch: Some(4),
        })
        .await
        .unwrap();
    assert_eq!(batch.batch, 5);
    assert_eq!(
        batch.events,
        [tinycomputer_bus::accessibility::GlobeEvent::Down]
    );
}

#[test]
fn successful_unit_response_preserves_its_explicit_null_data() {
    let raw = serde_json::to_value(DesktopResponse::ok(
        "accessibility-paste",
        serde_json::Value::Null,
    ))
    .unwrap();
    decode::<()>(raw).unwrap();
}

#[test]
fn successful_response_requires_the_data_field_even_for_unit_results() {
    assert!(matches!(
        decode::<()>(
            serde_json::json!({"version":"2.5", "command":"accessibility-paste", "ok":true})
        ),
        Err(AccessibilityError::Bus(ModuleCallError::ModuleFault))
    ));
}

#[test]
fn structured_focus_refusal_is_preserved_for_host_presentation() {
    let raw = serde_json::to_value(DesktopResponse::err(
        "accessibility-paste",
        DesktopError::new("FOCUS_CHANGED", "target changed"),
    ))
    .unwrap();
    assert!(
        matches!(decode::<()>(raw), Err(AccessibilityError::Operation(error)) if error.code == "FOCUS_CHANGED")
    );
}

#[cfg(all(feature = "modules", target_os = "linux"))]
#[tokio::test]
#[ignore = "requires a TinyComputer library: node scripts/modules/build.mjs tinycomputer, then OPENHUMAN_MODULE_SOURCE_DIR"]
async fn released_module_serves_permissions_and_confidential_target_validation() {
    let artifact = crate::modules::local_artifact("tinycomputer").expect(
        "build tinycomputer with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR",
    );
    let mut config = Config::default();
    config.modules.allow_download = false;
    config
        .modules
        .overrides
        .push(crate::config::schema::ModuleOverride::with_path(
            "tinycomputer",
            artifact.to_str().unwrap(),
        ));
    let client = AccessibilityClient::new(&config);
    let permissions = client.permissions().await.expect("native permission query");
    assert_eq!(permissions.accessibility, PermissionState::Unsupported);
    client
        .validate_target(FocusTarget::default())
        .await
        .expect("confidential null-result validation");
    assert!(matches!(client.focus(FocusQuery { verbose: true }).await,
        Err(AccessibilityError::Operation(error)) if error.code == "PLATFORM_NOT_SUPPORTED"));
    assert!(
        matches!(client.validate_target(FocusTarget { app: Some("fixture-editor".into()), ..Default::default() }).await,
        Err(AccessibilityError::Operation(error)) if error.code == "PLATFORM_NOT_SUPPORTED")
    );
    assert!(matches!(client.globe_start().await,
        Err(AccessibilityError::Operation(error)) if error.code == "PLATFORM_NOT_SUPPORTED"));
}

#[tokio::test]
async fn component_off_refuses_before_reaching_the_module() {
    let mut config = Config::default();
    config.components.computer = false;
    let client = AccessibilityClient::new(&config);
    assert!(!client.computer_enabled());
    match client.permissions().await {
        Err(AccessibilityError::Operation(error)) => {
            assert_eq!(error.message, "COMPONENT_DISABLED:computer");
        }
        other => panic!("expected COMPONENT_DISABLED, got {other:?}"),
    }
}
