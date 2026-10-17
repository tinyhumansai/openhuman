//! Tests for [`emit_external_transfer`](super::emit_external_transfer) — proves
//! external transfers publish an [`ExternalTransferPending`] event, local
//! transfers do not, and the turn's chat routing is attached (privacy epic S2,
//! #4436).

use super::super::{DataKind, EgressDescriptor, EgressReason, IdentificationRisk};
use super::*;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::approval::ApprovalChatContext;

/// Drain `rx` until an `ExternalTransferPending` whose descriptor `service`
/// matches `marker` arrives, returning it. Tolerates unrelated events and
/// broadcast lag (the bus is process-wide and other tests publish on it).
async fn find_pending(
    rx: &mut tinybus::events::EventReceiver<DomainEvent>,
    marker: &str,
) -> (EgressDescriptor, Option<String>, Option<String>) {
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending {
                descriptor,
                thread_id,
                client_id,
                ..
            }) if descriptor.service == marker => return (descriptor, thread_id, client_id),
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

#[tokio::test]
async fn external_transfer_publishes_pending_event() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "svc-external-emit-test";
    emit_external_transfer(None, EgressDescriptor::inference("openai", marker, true));

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
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let local_marker = "svc-local-emit-test";
    let sentinel_marker = "svc-sentinel-emit-test";
    // A local (non-external) transfer must NOT emit; a following external one
    // must. If we reach the sentinel without having seen the local marker, the
    // local transfer was correctly suppressed.
    emit_external_transfer(
        None,
        EgressDescriptor::inference("ollama", local_marker, false),
    );
    emit_external_transfer(None, EgressDescriptor::network_fetch(sentinel_marker));

    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                assert_ne!(
                    descriptor.service, local_marker,
                    "local (non-external) transfer must not publish ExternalTransferPending"
                );
                if descriptor.service == sentinel_marker {
                    break; // reached the sentinel without seeing the local marker
                }
            }
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

#[tokio::test]
async fn attaches_the_chat_it_is_given() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "svc-chat-context-emit-test";
    let chat = ApprovalChatContext {
        thread_id: "thread-xyz".to_string(),
        client_id: "client-abc".to_string(),
        request_id: None,
    };
    emit_external_transfer(Some(&chat), EgressDescriptor::composio(marker));

    let (descriptor, thread_id, client_id) = find_pending(&mut rx, marker).await;
    assert_eq!(descriptor.reason, EgressReason::ToolCall);
    assert_eq!(thread_id.as_deref(), Some("thread-xyz"));
    assert_eq!(client_id.as_deref(), Some("client-abc"));
}

/// Inside a [`EgressLedger::per_turn`], repeated disclosures of the *same* destination
/// (provider/service/reason) collapse to a single event, while a distinct
/// destination still publishes — the managed-turn fan-out fix (codex P2, #4812).
#[tokio::test]
async fn per_turn_ledger_collapses_repeat_destination() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let dup = "svc-dedup-dup-test";
    let distinct = "svc-dedup-distinct-test";
    let sentinel = "svc-dedup-sentinel-test";
    let ledger = EgressLedger::per_turn();
    emit_external_transfer_in(&ledger, EgressDescriptor::inference("openhuman", dup, true));
    // Same destination again — must be suppressed within the turn.
    emit_external_transfer_in(&ledger, EgressDescriptor::inference("openhuman", dup, true));
    // A distinct destination (different service) still discloses.
    emit_external_transfer_in(
        &ledger,
        EgressDescriptor::inference("openhuman", distinct, true),
    );
    // Sentinel closes the drain window.
    emit_external_transfer_in(&ledger, EgressDescriptor::network_fetch(sentinel));

    let mut dup_count = 0;
    let mut distinct_seen = false;
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
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
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
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

/// Outside any [`EgressLedger::per_turn`] (CLI / cron / a single egress site) there is
/// no ledger, so every `emit_external_transfer` publishes — dedup never leaks
/// across unrelated calls.
#[tokio::test]
async fn dedup_absent_outside_scope_publishes_each_time() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "svc-nodedup-test";
    let sentinel = "svc-nodedup-sentinel-test";
    emit_external_transfer(None, EgressDescriptor::inference("openhuman", marker, true));
    emit_external_transfer(None, EgressDescriptor::inference("openhuman", marker, true));
    emit_external_transfer(None, EgressDescriptor::network_fetch(sentinel));

    let mut count = 0;
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                if descriptor.service == marker {
                    count += 1;
                }
                if descriptor.service == sentinel {
                    break;
                }
            }
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
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
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "svc-risk-emit-test";
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::composio(marker)
            .with_risk(IdentificationRisk::High, vec!["email".to_string()]),
        thread_id: None,
        client_id: None,
        request_id: None,
    });

    let (descriptor, _, _) = find_pending(&mut rx, marker).await;
    assert_eq!(descriptor.risk_level, IdentificationRisk::High);
    assert_eq!(descriptor.risk_categories, vec!["email"]);
}

fn chat(thread: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: Some(format!("req-{thread}")),
    }
}

/// A ledger carries the turn's chat to every disclosure raised through it, so
/// a disclosure made deep in the provider factory chain still reaches the chat
/// that triggered it, and two turns' ledgers never mix.
#[tokio::test]
async fn a_routed_ledger_attaches_its_chat_to_every_disclosure() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let ledger_a = EgressLedger::per_turn().routed_to(Some(chat("thread-a")));
    let ledger_b = EgressLedger::per_turn().routed_to(Some(chat("thread-b")));
    emit_external_transfer_in(
        &ledger_a,
        EgressDescriptor::inference("openhuman", "svc-routed-ledger-a", true),
    );
    // An await point between the two turns' disclosures.
    tokio::task::yield_now().await;
    emit_external_transfer_in(
        &ledger_b,
        EgressDescriptor::inference("openhuman", "svc-routed-ledger-b", true),
    );

    let (_, thread_a, client_a) = find_pending(&mut rx, "svc-routed-ledger-a").await;
    assert_eq!(thread_a.as_deref(), Some("thread-a"));
    assert_eq!(client_a.as_deref(), Some("client-thread-a"));
    let (_, thread_b, client_b) = find_pending(&mut rx, "svc-routed-ledger-b").await;
    assert_eq!(thread_b.as_deref(), Some("thread-b"));
    assert_eq!(client_b.as_deref(), Some("client-thread-b"));
}

#[tokio::test]
async fn a_disclosure_outside_a_chat_turn_is_unrouted() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    emit_external_transfer_in(
        &EgressLedger::per_turn(),
        EgressDescriptor::inference("openhuman", "svc-unrouted-ledger", true),
    );
    let (_, thread_id, client_id) = find_pending(&mut rx, "svc-unrouted-ledger").await;
    assert_eq!((thread_id, client_id), (None, None));
}
