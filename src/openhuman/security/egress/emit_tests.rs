//! Tests for [`emit_external_transfer`](super::emit_external_transfer) — proves
//! external transfers publish an [`ExternalTransferPending`] event, local
//! transfers do not, and ambient chat context is attached (privacy epic S2,
//! #4436).

use super::super::{DataKind, EgressDescriptor, EgressReason, IdentificationRisk};
use super::*;
use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
use crate::openhuman::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};

/// Drain `rx` until an `ExternalTransferPending` whose descriptor `service`
/// matches `marker` arrives, returning it. Tolerates unrelated events and
/// broadcast lag (the bus is process-wide and other tests publish on it).
async fn find_pending(
    rx: &mut tokio::sync::broadcast::Receiver<DomainEvent>,
    marker: &str,
) -> (EgressDescriptor, Option<String>, Option<String>) {
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ExternalTransferPending {
                descriptor,
                thread_id,
                client_id,
            }) if descriptor.service == marker => return (descriptor, thread_id, client_id),
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                panic!("event bus closed before ExternalTransferPending arrived")
            }
        }
    }
}

#[tokio::test]
async fn external_transfer_publishes_pending_event() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let marker = "svc-external-emit-test";
    emit_external_transfer(EgressDescriptor::inference("openai", marker, true));

    let (descriptor, thread_id, client_id) = find_pending(&mut rx, marker).await;
    assert_eq!(descriptor.provider_slug, "openai");
    assert!(descriptor.is_external);
    assert_eq!(descriptor.reason, EgressReason::Inference);
    assert_eq!(descriptor.data_kinds, vec![DataKind::Prompt]);
    // No ambient chat context in this test task → no routing.
    assert_eq!(thread_id, None);
    assert_eq!(client_id, None);
}

#[tokio::test]
async fn local_transfer_does_not_publish() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let local_marker = "svc-local-emit-test";
    let sentinel_marker = "svc-sentinel-emit-test";
    // A local (non-external) transfer must NOT emit; a following external one
    // must. If we reach the sentinel without having seen the local marker, the
    // local transfer was correctly suppressed.
    emit_external_transfer(EgressDescriptor::inference("ollama", local_marker, false));
    emit_external_transfer(EgressDescriptor::network_fetch(sentinel_marker));

    loop {
        match rx.recv().await {
            Ok(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                assert_ne!(
                    descriptor.service, local_marker,
                    "local (non-external) transfer must not publish ExternalTransferPending"
                );
                if descriptor.service == sentinel_marker {
                    break; // reached the sentinel without seeing the local marker
                }
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                panic!("bus closed before sentinel arrived")
            }
        }
    }
}

#[tokio::test]
async fn attaches_ambient_chat_context() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let marker = "svc-chat-context-emit-test";
    APPROVAL_CHAT_CONTEXT
        .scope(
            ApprovalChatContext {
                thread_id: "thread-xyz".to_string(),
                client_id: "client-abc".to_string(),
            },
            async {
                emit_external_transfer(EgressDescriptor::composio(marker));
            },
        )
        .await;

    let (descriptor, thread_id, client_id) = find_pending(&mut rx, marker).await;
    assert_eq!(descriptor.reason, EgressReason::ToolCall);
    assert_eq!(thread_id.as_deref(), Some("thread-xyz"));
    assert_eq!(client_id.as_deref(), Some("client-abc"));
}

/// The event carries the S5 risk fields verbatim so a future detector arm can
/// attach a risk level without reshaping the event.
#[tokio::test]
async fn carries_risk_fields_when_present() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let marker = "svc-risk-emit-test";
    publish_global(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::composio(marker)
            .with_risk(IdentificationRisk::High, vec!["email".to_string()]),
        thread_id: None,
        client_id: None,
    });

    let (descriptor, _, _) = find_pending(&mut rx, marker).await;
    assert_eq!(descriptor.risk_level, IdentificationRisk::High);
    assert_eq!(descriptor.risk_categories, vec!["email"]);
}
