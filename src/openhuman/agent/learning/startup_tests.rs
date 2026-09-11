use super::*;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::learning::candidate::Buffer;
use crate::openhuman::agent::learning::extract::signature::{
    parse_signature, register_email_signature_subscriber_on,
};
use std::time::Duration;
use tempfile::TempDir;

/// A fresh temp workspace for the ready-arm test.
///
/// This used to build a real `MemoryClient` and hand it to
/// `register_with_client`, which never used it for anything but its
/// presence. Readiness is a `bool` now (#5560), so the fixture is just the
/// directory — but the host seams still have to be installed, because
/// `memory_is_bindable` and the facet cache below both resolve a driver and
/// an unwired embedding host fails loudly by design. `install_for_tests` is
/// `Once`-guarded, so calling it here is free when another test already has.
fn test_workspace() -> TempDir {
    TempDir::new().expect("tempdir")
}

/// A body whose trailing lines form a clear email signature — yields several
/// Identity candidates (name/role/timezone/employer).
fn signature_body() -> String {
    "Hi, great to hear from you!\n\n\
     Thanks,\n\
     Alice Johnson\n\
     Senior Software Engineer\n\
     Acme Corp\n\
     San Francisco, CA\n\
     PST"
    .to_string()
}

fn email_doc(source_id: &str, body: &str) -> DomainEvent {
    DomainEvent::DocumentCanonicalized {
        source_id: source_id.to_string(),
        source_kind: "email".to_string(),
        chunks_written: 1,
        chunk_ids: vec![format!("{source_id}-c1")],
        canonicalized_at: 0.0,
        body_preview: Some(body.to_string()),
    }
}

/// Poll an isolated buffer until at least `expected` candidates appear,
/// then settle briefly so an accidental duplicate subscription surfaces.
async fn wait_for_candidates(buffer: &Buffer, expected: usize) -> usize {
    for _ in 0..50 {
        if buffer.len() >= expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    buffer.len()
}

#[tokio::test]
async fn register_with_memory_registers_both_handles_when_ready() {
    crate::core::bus::init().await.expect("bus init");
    let tmp = test_workspace();
    let (trigger, renderer) = register_with_memory(true, tmp.path());
    assert!(
        trigger.is_some(),
        "rebuild trigger must register when memory is available"
    );
    assert!(
        renderer.is_some(),
        "ProfileMdRenderer must register when memory is available"
    );
}

#[tokio::test]
async fn register_with_memory_skips_and_warns_when_memory_absent() {
    // No memory driver → both memory-dependent subscribers are skipped and
    // the (now loud) warn path is exercised. This is the else-arm the #5003
    // fix upgraded from a silent debug-level skip.
    let tmp = TempDir::new().expect("tempdir");
    let (trigger, renderer) = register_with_memory(false, tmp.path());
    assert!(trigger.is_none(), "no trigger without a memory driver");
    assert!(renderer.is_none(), "no renderer without a memory driver");
}

#[tokio::test]
async fn learning_subscriber_fires_with_no_channel_configured() {
    let bus = crate::core::bus_testing::isolated_bus().await;
    let buffer: &'static Buffer = Box::leak(Box::new(Buffer::new(16)));
    let handle_cell = OnceLock::new();
    register_email_signature_once(&handle_cell, || {
        Some(register_email_signature_subscriber_on(&bus, buffer))
    });

    let source_id = "gmail:5003-e2e";
    let body = signature_body();
    let expected = parse_signature(&body, source_id, source_id).len();
    assert!(
        expected > 0,
        "signature body must yield at least one identity candidate"
    );

    bus.publish(email_doc(source_id, &body));
    let got = wait_for_candidates(buffer, expected).await;
    assert_eq!(
        got, expected,
        "email-signature subscriber must push the parsed identity candidates \
         with no channel configured anywhere (#5003)"
    );
}

#[tokio::test]
async fn register_learning_subscribers_is_idempotent() {
    let bus = crate::core::bus_testing::isolated_bus().await;
    let buffer: &'static Buffer = Box::leak(Box::new(Buffer::new(16)));
    let handle_cell = OnceLock::new();
    register_email_signature_once(&handle_cell, || {
        Some(register_email_signature_subscriber_on(&bus, buffer))
    });
    register_email_signature_once(&handle_cell, || {
        Some(register_email_signature_subscriber_on(&bus, buffer))
    });

    let source_id = "gmail:5003-idem";
    let body = signature_body();
    let expected = parse_signature(&body, source_id, source_id).len();
    assert!(expected > 0);

    bus.publish(email_doc(source_id, &body));
    let got = wait_for_candidates(buffer, expected).await;
    assert_eq!(
        got, expected,
        "double registration must not double the pushed candidates (#5003 idempotency)"
    );
}
