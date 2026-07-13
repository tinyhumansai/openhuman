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

/// Inside a [`dedup_turn_scope`], repeated disclosures of the *same* destination
/// (provider/service/reason) collapse to a single event, while a distinct
/// destination still publishes — the managed-turn fan-out fix (codex P2, #4812).
#[tokio::test]
async fn dedup_turn_scope_collapses_repeat_destination() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let dup = "svc-dedup-dup-test";
    let distinct = "svc-dedup-distinct-test";
    let sentinel = "svc-dedup-sentinel-test";
    dedup_turn_scope(|| {
        emit_external_transfer(EgressDescriptor::inference("openhuman", dup, true));
        // Same destination again — must be suppressed within the turn.
        emit_external_transfer(EgressDescriptor::inference("openhuman", dup, true));
        // A distinct destination (different service) still discloses.
        emit_external_transfer(EgressDescriptor::inference("openhuman", distinct, true));
        // Sentinel closes the drain window.
        emit_external_transfer(EgressDescriptor::network_fetch(sentinel));
    });

    let mut dup_count = 0;
    let mut distinct_seen = false;
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                if descriptor.service == dup {
                    dup_count += 1;
                }
                if descriptor.service == distinct {
                    distinct_seen = true;
                }
                if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                panic!("bus closed before sentinel arrived")
            }
        }
    }
    assert_eq!(
        dup_count, 1,
        "a repeated destination must disclose exactly once per turn"
    );
    assert!(
        distinct_seen,
        "a distinct destination must still disclose within the same turn"
    );
}

/// Outside any [`dedup_turn_scope`] (CLI / cron / a single egress site) there is
/// no ledger, so every `emit_external_transfer` publishes — dedup never leaks
/// across unrelated calls.
#[tokio::test]
async fn dedup_absent_outside_scope_publishes_each_time() {
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let marker = "svc-nodedup-test";
    let sentinel = "svc-nodedup-sentinel-test";
    emit_external_transfer(EgressDescriptor::inference("openhuman", marker, true));
    emit_external_transfer(EgressDescriptor::inference("openhuman", marker, true));
    emit_external_transfer(EgressDescriptor::network_fetch(sentinel));

    let mut count = 0;
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                if descriptor.service == marker {
                    count += 1;
                }
                if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                panic!("bus closed before sentinel arrived")
            }
        }
    }
    assert_eq!(
        count, 2,
        "without a dedup scope each identical transfer must publish"
    );
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
