use super::*;

#[tokio::test]
async fn disabled_processing_never_executes_linked_calendar_or_validation_helpers() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let result = prepare(
        &config,
        "GOOGLECALENDAR_LIST_EVENTS",
        Some(serde_json::json!({})),
        Some("UTC".into()),
        None,
    )
    .await;
    assert!(result
        .unwrap_err()
        .contains("MODULE_CALL_REPORTED: module unavailable"));
}

#[cfg(feature = "modules")]
#[derive(Clone)]
struct Fixture(std::sync::Arc<Mutex<Vec<PrepareArgumentsRequest>>>);
#[cfg(feature = "modules")]
use tokio::sync::Mutex;

#[cfg(feature = "modules")]
#[tinybus::interface(name = "test.ConnectorProcessing")]
impl Fixture {
    async fn prepare_arguments(
        &self,
        request: PrepareArgumentsRequest,
    ) -> tinybus::Result<PreparedArguments> {
        self.0.lock().await.push(request.clone());
        Ok(PreparedArguments {
            arguments: Some(serde_json::json!({"module_prepared":true})),
            error: (request.tool == "invalid").then(|| tinyconnectors_bus::ProviderError {
                class: "validation".into(),
                message: "provider validation refused the action".into(),
            }),
        })
    }
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn preparation_forwards_host_context_and_preserves_module_validation_errors() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
    service
        .serve_at(
            ObjectPath::new("/test/processing").unwrap(),
            Fixture(requests.clone()),
        )
        .await
        .unwrap();
    service
        .request_name("test.ConnectorProcessing")
        .await
        .unwrap();
    let client = crate::modules::client::ModuleClient::fixture(
        caller
            .proxy(
                "test.ConnectorProcessing",
                "/test/processing",
                "test.ConnectorProcessing",
            )
            .unwrap(),
    );
    let args = Some(serde_json::json!({"caller_argument":"value"}));
    assert_eq!(
        prepare_with(
            &client,
            "calendar",
            args.clone(),
            Some("Asia/Kuwait".into()),
            Some("2026-10-10T00:00:00Z".into())
        )
        .await
        .unwrap(),
        Some(serde_json::json!({"module_prepared":true}))
    );
    let captured = requests.lock().await;
    assert_eq!(captured[0].arguments, args);
    assert_eq!(captured[0].timezone.as_deref(), Some("Asia/Kuwait"));
    assert_eq!(captured[0].since.as_deref(), Some("2026-10-10T00:00:00Z"));
    drop(captured);
    assert_eq!(
        prepare_with(&client, "invalid", None, None, None)
            .await
            .unwrap_err(),
        "provider validation refused the action"
    );
}
