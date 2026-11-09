use super::*;
use tinystoragedrivers_busproxy::{Object, StorageCall};

async fn endpoint() -> (DocumentsOnlyEndpoint, BusName) {
    let backend = crate::storage::test_support::memory_backend();
    let scope = crate::storage::Scope::new("local").unwrap();
    let storage = backend.for_scope(&scope).unwrap();
    storage
        .documents()
        .ensure_collection(&crate::storage::CollectionSpec::new("juice.savings"))
        .await
        .unwrap();
    let expected_peer = BusName::new(":1.7").unwrap();
    (
        DocumentsOnlyEndpoint {
            expected_peer: expected_peer.clone(),
            active: Arc::new(AtomicBool::new(true)),
            grant: crate::modules::storage_grants::StorageGrant {
                collections: vec![crate::storage::CollectionSpec::new("juice.savings")],
                writable: true,
                ..crate::modules::storage_grants::StorageGrant::default()
            },
            server: StorageServer::new(storage),
        },
        expected_peer,
    )
}

async fn call(
    endpoint: &DocumentsOnlyEndpoint,
    sender: Option<&BusName>,
    object: Object,
    selector: &str,
    request: Value,
) -> Result<Value, HostFailure> {
    endpoint
        .call(StorageCall {
            object,
            selector,
            request,
            sender,
            admission: Admission::Private,
        })
        .await
}

#[tokio::test]
async fn callback_authenticates_before_metadata_and_restricts_ports() {
    let (endpoint, peer) = endpoint().await;
    let attacker = BusName::new(":1.8").unwrap();

    assert_eq!(
        call(
            &endpoint,
            Some(&attacker),
            Object::Streams,
            "Capabilities",
            json!(null),
        )
        .await,
        Err(HostFailure::Denied("wrong-module-instance"))
    );
    assert_eq!(
        call(&endpoint, None, Object::Blobs, "Capabilities", json!(null),).await,
        Err(HostFailure::Denied("unauthenticated-caller"))
    );

    let stream_caps = call(
        &endpoint,
        Some(&peer),
        Object::Streams,
        "Capabilities",
        json!(null),
    )
    .await
    .unwrap();
    assert_eq!(stream_caps, json!({ "capabilities": [] }));
    assert_eq!(
        call(
            &endpoint,
            Some(&peer),
            Object::Blobs,
            "Get",
            json!({"key": "secret"}),
        )
        .await,
        Err(HostFailure::Denied("documents-only-invocation"))
    );
    assert_eq!(
        call(
            &endpoint,
            Some(&peer),
            Object::Secrets,
            "Capabilities",
            json!(null),
        )
        .await,
        Err(DocumentsOnlyEndpoint::unsupported())
    );
}

#[tokio::test]
async fn callback_documents_route_uses_the_captured_storage() {
    let (endpoint, peer) = endpoint().await;

    let result = call(
        &endpoint,
        Some(&peer),
        Object::Documents,
        "Get",
        json!({"collection": "juice.savings", "document_id": "current"}),
    )
    .await;

    assert_eq!(result, Ok(Value::Null));
}

#[tokio::test]
async fn callback_documents_route_keeps_the_module_collection_grant() {
    let (endpoint, peer) = endpoint().await;

    assert_eq!(
        call(
            &endpoint,
            Some(&peer),
            Object::Documents,
            "Get",
            json!({"collection": "another.module.collection", "document_id": "x"}),
        )
        .await,
        Err(HostFailure::Denied("target-not-granted"))
    );
}

#[tokio::test]
async fn callback_documents_route_does_not_advertise_or_open_unrelayed_watches() {
    let (endpoint, peer) = endpoint().await;

    let capabilities = call(
        &endpoint,
        Some(&peer),
        Object::Documents,
        "Capabilities",
        json!(null),
    )
    .await
    .unwrap();
    assert!(!capabilities["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|capability| capability == "watch"));
    assert_eq!(
        call(
            &endpoint,
            Some(&peer),
            Object::Documents,
            "WatchOpen",
            json!({
                "collection": "juice.savings",
                "filter": {"kind": "all"},
                "interval_ms": 10
            }),
        )
        .await,
        Err(DocumentsOnlyEndpoint::unsupported())
    );
}

#[tokio::test]
async fn dropped_invocation_refuses_calls_already_in_flight_at_endpoint() {
    let (endpoint, peer) = endpoint().await;
    endpoint.active.store(false, Ordering::Release);

    assert_eq!(
        call(
            &endpoint,
            Some(&peer),
            Object::Documents,
            "Capabilities",
            json!(null),
        )
        .await,
        Err(HostFailure::Denied("storage-endpoint-closed"))
    );
}
