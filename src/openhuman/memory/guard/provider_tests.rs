//! The guard as a [`MemoryProvider`]: capability mirroring, the mandatory
//! three, and step 7's audit event.

use super::*;
use std::sync::Arc;

use crate::openhuman::memory::api::capabilities::{Capabilities, Capability};
use crate::openhuman::memory::api::provider::types::SourceScope;
use crate::openhuman::memory::api::provider::{
    audit_provider, MemoryCore, MemoryPortability, MemoryProvider, MemoryRecall,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{MemoryCategory, MemoryTaint};
use tinymemory_api::null::NullMemoryProvider;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::subsystem::DriverClass;
use crate::openhuman::config::schema::MemoryHooksConfig;
use crate::openhuman::memory::guard::policy::TRUSTED;
use crate::openhuman::memory::guard::test_support::{
    embedded_policy, entry, export_record, external_policy, guarded, guarded_with,
    RecordingProvider,
};
use crate::openhuman::memory::guard::GuardPolicy;
use crate::openhuman::memory::source_scope::with_source_scope;

fn budgeted(recall_max_chars: usize, capture_max_chars: usize) -> GuardPolicy {
    GuardPolicy::new(
        "recording",
        DriverClass::Embedded,
        MemoryHooksConfig {
            recall_max_chars,
            capture_max_chars,
            ..MemoryHooksConfig::default()
        },
        TRUSTED,
    )
}

// ── Identity + capability mirroring ─────────────────────────────────────────

#[tokio::test]
async fn guard_reports_the_wrapped_drivers_identity() {
    let (_driver, guard) = guarded(embedded_policy());
    assert_eq!(
        guard.driver_id(),
        "recording",
        "the guard is a policy layer, not a driver"
    );
    assert_eq!(guard.capabilities(), Capabilities::all());
}

#[tokio::test]
async fn guard_passes_audit_provider_against_its_own_capabilities() {
    let (_driver, guard) = guarded(embedded_policy());
    audit_provider(&guard).expect("advertised set and reachable accessors must agree");
}

#[tokio::test]
async fn guard_accessor_presence_mirrors_inner_provides_for_every_family() {
    let (_driver, guard) = guarded(embedded_policy());
    for capability in Capability::ALL {
        assert!(
            guard.provides(capability),
            "{capability} must be reachable through the guard"
        );
    }

    // The other direction: a driver with only the mandatory three must not
    // acquire families merely by being guarded.
    let inner = Arc::new(NullMemoryProvider::new());
    let null = MemoryGuard::new(
        Arc::clone(&inner) as Arc<dyn MemoryProvider>,
        Arc::new(embedded_policy()),
    );
    for capability in Capability::ALL {
        assert_eq!(
            null.provides(capability),
            inner.provides(capability),
            "{capability} presence must mirror the inner driver exactly"
        );
    }
    audit_provider(&null).expect("mandatory-only driver stays consistent when guarded");
}

// ── The mandatory three ─────────────────────────────────────────────────────

#[tokio::test]
async fn guard_stamps_taint_on_store_rather_than_trusting_the_caller() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .store(
                "ns",
                "k",
                "hello",
                MemoryCategory::Core,
                None,
                MemoryTaint::Internal,
            )
            .await
            .expect("store");
    })
    .await;
    assert_eq!(driver.only_call().taint, Some(MemoryTaint::ExternalSync));
}

#[tokio::test]
async fn guard_truncates_stored_content_to_capture_max_chars() {
    let (driver, guard) = guarded(budgeted(1000, 5));
    guard
        .store(
            "ns",
            "k",
            "hello world",
            MemoryCategory::Core,
            None,
            MemoryTaint::Internal,
        )
        .await
        .expect("store");
    assert_eq!(driver.only_call().content.as_deref(), Some("hello"));
}

#[tokio::test]
async fn guard_truncates_recall_results_to_recall_max_chars() {
    let (_driver, guard) = guarded_with(
        RecordingProvider::new().with_recall_result(vec![
            entry("aaaa"),
            entry("bbbb"),
            entry("cccc"),
        ]),
        budgeted(6, 500),
    );
    let hits = guard
        .recall("q", 10, &OwnedRecallOpts::default(), None)
        .await
        .expect("recall");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].content, "aaaa");
    assert_eq!(hits[1].content, "bb");
}

/// Pins the departure argued in `mandatory.rs`: the embedded driver *refuses* a
/// `Some(scope)` on recall, so the guard must NOT fill it from the task-local.
#[tokio::test]
async fn guard_never_fills_scope_on_recall() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .recall("q", 10, &OwnedRecallOpts::default(), None)
            .await
            .expect("recall must not become an error merely by being scoped");
    })
    .await;
    assert_eq!(driver.only_call().scoped, Some(false));
}

#[tokio::test]
async fn guard_forwards_an_explicit_recall_scope_untouched() {
    let (driver, guard) = guarded(embedded_policy());
    let scope = SourceScope::new(["slack:#eng"]);
    let _ = guard
        .recall("q", 10, &OwnedRecallOpts::default(), Some(&scope))
        .await;
    assert_eq!(driver.only_call().scoped, Some(true));
}

#[tokio::test]
async fn guard_preserves_import_taint_rather_than_restamping_it() {
    let (driver, guard) = guarded(embedded_policy());
    // Inside a source scope, so a naive "stamp everything" would show up.
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .import_records(vec![export_record(MemoryTaint::Internal)])
            .await
            .expect("import");
    })
    .await;
    assert_eq!(
        driver.only_call().taint,
        Some(MemoryTaint::Internal),
        "a restore must not have its provenance rewritten wholesale"
    );
}

#[tokio::test]
async fn guard_does_not_budget_trim_an_export() {
    let (driver, guard) = guarded(budgeted(1, 1));
    guard.export_page(None, 10).await.expect("export");
    assert_eq!(driver.only_call().method, "portability.export_page");
}

// ── Step 7 ──────────────────────────────────────────────────────────────────

struct DeniedRecorder {
    seen: std::sync::Mutex<Vec<(String, String, String)>>,
}

#[async_trait::async_trait]
impl tinybus::EventHandler<DomainEvent> for DeniedRecorder {
    fn name(&self) -> &str {
        "memory::guard::test_recorder"
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::MemoryGuardDenied {
            driver_id,
            method,
            reason,
        } = event
        {
            self.seen.lock().expect("recorder mutex").push((
                driver_id.clone(),
                method.clone(),
                reason.clone(),
            ));
        }
    }
}

async fn record_denials() -> (Arc<DeniedRecorder>, tinybus::SubscriptionHandle) {
    crate::core::bus::init().await.expect("bus init");
    let recorder = Arc::new(DeniedRecorder {
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let handle = BUS
        .subscribe(recorder.clone())
        .expect("the bus was just initialised");
    (recorder, handle)
}

async fn await_denial(recorder: &DeniedRecorder, driver_id: &str) -> (String, String, String) {
    for _ in 0..200 {
        if let Some(found) = recorder
            .seen
            .lock()
            .expect("recorder mutex")
            .iter()
            .find(|(id, _, _)| id == driver_id)
        {
            return found.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no MemoryGuardDenied event for driver '{driver_id}' within 2s");
}

#[tokio::test]
async fn guard_publishes_memory_guard_denied_on_refusal() {
    let (recorder, _handle) = record_denials().await;
    let (driver, guard) = guarded(external_policy("untrusted"));
    let err = guard
        .store(
            "ns",
            "k",
            "hello",
            MemoryCategory::Core,
            None,
            MemoryTaint::Internal,
        )
        .await
        .expect_err("untrusted external driver must be refused");
    assert!(err.to_string().contains("memory guard: "));
    assert_eq!(driver.call_count(), 0, "the driver must never be reached");

    let (driver_id, method, reason) = await_denial(&recorder, "supermemory").await;
    assert_eq!(driver_id, "supermemory");
    assert_eq!(method, "core.store");
    assert!(!reason.contains("hello"), "must never carry content");
}

#[tokio::test]
async fn guard_publishes_nothing_on_the_success_path() {
    let (recorder, _handle) = record_denials().await;
    let (_driver, guard) = guarded(embedded_policy());
    guard
        .store(
            "ns",
            "k",
            "hello",
            MemoryCategory::Core,
            None,
            MemoryTaint::Internal,
        )
        .await
        .expect("store");
    guard
        .recall("q", 3, &OwnedRecallOpts::default(), None)
        .await
        .expect("recall");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let seen = recorder.seen.lock().expect("recorder mutex");
    assert!(
        !seen
            .iter()
            .any(|(driver_id, _, _)| driver_id == "recording"),
        "a guarded read/write must not publish on success, saw: {seen:?}"
    );
}
