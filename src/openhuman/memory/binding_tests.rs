//! Tests for the per-workspace memory-driver binding.
//!
//! The load-bearing ones are the trust pair (`admit_refuses_untrusted_external_driver`
//! / `admit_refuses_trusted_external_driver_until_transport_exists`) and
//! `capabilities_are_asked_exactly_once_per_bind`. The first two are written so
//! neither can pass for the other's reason; the third pins the contract's
//! "asked once at bind time and cached" rule, which the whole capability gate
//! depends on.

use super::*;
use crate::core::subsystem::DriverClass;
use crate::openhuman::config::schema::MemorySubsystemConfig;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// `binding.rs` reaches these through its own `use` statements; a sibling test
// module only inherits its `pub` items, so they are named again here.
use crate::core::subsystem::{DriverHealth, SubsystemSlot};
use tinycortex_api::capabilities::Capabilities;
use tinycortex_api::health::MemoryHealth;
use tinycortex_api::null::{NullMemoryProvider, NULL_DRIVER_ID};
use tinycortex_api::provider::MemoryProvider;
use tinycortex_api::CONTRACT_VERSION;

// Imported here rather than re-exported from `binding.rs`: since admission
// moved to `tinymemory::registry`, the production module no longer names this
// constant and an import kept alive only for the tests would read as dead code.
use crate::openhuman::memory::driver::embedded::EMBEDDED_DRIVER_ID;

use async_trait::async_trait;
use tinycortex_api::capabilities::Capability;
use tinycortex_api::error::MemoryError;
use tinycortex_api::provider::types::{ExportPage, ExportRecord, ImportOutcome, SourceScope};
use tinycortex_api::provider::{MemoryCore, MemoryPortability, MemoryRecall};
use tinycortex_api::recall::OwnedRecallOpts;
use tinycortex_api::types::{MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary};

use tinymemory_api::host::MemoryDriverConfig;

fn external_driver_cfg(trust_state: &str) -> MemorySubsystemConfig {
    let mut cfg = MemorySubsystemConfig {
        driver: "supermemory".into(),
        ..Default::default()
    };
    cfg.drivers.insert(
        "supermemory".into(),
        MemoryDriverConfig {
            class: Some("external".into()),
            transport: Some("http".into()),
            endpoint: Some("https://api.supermemory.ai".into()),
            credential_ref: Some("keychain:supermemory".into()),
            trust_state: trust_state.into(),
        },
    );
    cfg
}

#[test]
fn admit_default_config_binds_embedded_tinycortex() {
    let (id, class) = admit(&MemorySubsystemConfig::default()).expect("default config admits");
    assert_eq!(id, "tinycortex");
    assert_eq!(class, DriverClass::Embedded);
}

#[test]
fn admit_null_driver_binds_null_class() {
    let cfg = MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };
    let (id, class) = admit(&cfg).expect("null driver admits");
    assert_eq!(id, "null");
    assert_eq!(class, DriverClass::Null);
}

#[test]
fn admit_typo_d_embedded_driver_id_gets_embedded_class() {
    // Regression for the reviewer finding: before this, any non-null id without
    // a drivers entry — a typo like "tinycortx", or an external backend that
    // forgot its table — was silently classified Embedded. Only the two built-in
    // ids admit implicitly.
    let cfg = MemorySubsystemConfig {
        driver: "tinycortex".into(),
        ..Default::default()
    };
    let (id, class) = admit(&cfg).expect("the embedded default id admits");
    assert_eq!(id, "tinycortex");
    assert_eq!(class, DriverClass::Embedded);
}

#[test]
fn admit_refuses_an_unregistered_non_null_driver_id() {
    // A typo or an external backend with no `drivers.<id>` entry must not
    // silently run the embedded engine under an invented driver id.
    let cfg = MemorySubsystemConfig {
        driver: "supermemory".into(),
        ..Default::default()
    };
    let refusal = admit(&cfg).expect_err("an unregistered id must be refused");
    assert_eq!(refusal.configured_driver, "supermemory");
    assert!(
        refusal.reason.contains("supermemory"),
        "refusal must name the offending id: {}",
        refusal.reason
    );
    assert!(
        refusal.reason.contains("drivers"),
        "refusal must point at the missing drivers table: {}",
        refusal.reason
    );
}

#[test]
fn admit_refuses_non_builtin_id_even_with_a_drivers_entry_that_says_no_class() {
    // Same rule when an entry exists but carries no `class` line: only the two
    // built-in ids imply a class. An arbitrary id must not silently become
    // Embedded just because someone registered a placeholder entry.
    let mut cfg = MemorySubsystemConfig {
        driver: "custom-mem".into(),
        ..Default::default()
    };
    cfg.drivers.insert(
        "custom-mem".into(),
        MemoryDriverConfig {
            class: None,
            ..Default::default()
        },
    );
    let refusal = admit(&cfg).expect_err("entry with no class must not admit an arbitrary id");
    assert_eq!(refusal.configured_driver, "custom-mem");
    assert!(
        refusal.reason.contains("custom-mem"),
        "refusal must name the offending id: {}",
        refusal.reason
    );
    assert!(
        refusal.reason.contains("class line"),
        "refusal must point at the missing class line: {}",
        refusal.reason
    );
}

#[test]
fn admit_accepts_an_explicit_embedded_class_for_a_registered_id() {
    // A drivers entry that explicitly names the embedded class is a deliberate
    // declaration — that id genuinely means the in-process engine. Explicit
    // beats implicit.
    let mut cfg = MemorySubsystemConfig {
        driver: "custom-mem".into(),
        ..Default::default()
    };
    cfg.drivers.insert(
        "custom-mem".into(),
        MemoryDriverConfig {
            class: Some("embedded".into()),
            ..Default::default()
        },
    );
    let (id, class) = admit(&cfg).expect("explicit embedded class admits");
    assert_eq!(id, "custom-mem");
    assert_eq!(class, DriverClass::Embedded);
}

#[test]
fn admit_refuses_untrusted_external_driver() {
    // The default trust_state is "untrusted" (kernel.md §3.4, fail-closed).
    let cfg = external_driver_cfg(&MemoryDriverConfig::default().trust_state);
    let refusal = admit(&cfg).expect_err("untrusted external driver must be refused");
    assert_eq!(refusal.configured_driver, "supermemory");
    assert!(
        refusal.reason.contains("trust_state"),
        "refusal must name the trust rule: {}",
        refusal.reason
    );
}

#[test]
fn admit_refuses_trusted_external_driver_until_transport_exists() {
    let cfg = external_driver_cfg("trusted");
    let refusal = admit(&cfg).expect_err("no external transport exists yet");
    assert!(
        refusal.reason.contains("transport"),
        "refusal must name the missing transport: {}",
        refusal.reason
    );
    assert!(
        !refusal.reason.contains("trust_state"),
        "a trusted driver must not be refused for trust: {}",
        refusal.reason
    );
}

#[test]
fn admit_rejects_an_unknown_driver_class() {
    let mut cfg = external_driver_cfg("trusted");
    cfg.drivers.get_mut("supermemory").unwrap().class = Some("embeded".into());
    let refusal = admit(&cfg).expect_err("typo'd class must be refused");
    assert!(
        refusal.reason.contains("embeded"),
        "refusal must echo the typo: {}",
        refusal.reason
    );
}

#[test]
fn fallback_reason_never_contains_credential_ref_or_endpoint() {
    let mut cfg = external_driver_cfg("untrusted");
    cfg.drivers.get_mut("supermemory").unwrap().credential_ref =
        Some("keychain:super-secret-value".into());
    let refusal = admit(&cfg).expect_err("untrusted external driver must be refused");
    assert!(
        !refusal.reason.contains("super-secret-value"),
        "credential_ref leaked into an operator-facing string: {}",
        refusal.reason
    );
    assert!(
        !refusal.reason.contains("supermemory.ai"),
        "endpoint leaked into an operator-facing string: {}",
        refusal.reason
    );
}

#[test]
fn for_workspace_caches_binding_per_workspace() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let cfg = MemorySubsystemConfig::default();

    let a = for_workspace(dir_a.path(), &cfg).expect("bind workspace A");
    let b = for_workspace(dir_b.path(), &cfg).expect("bind workspace B");
    assert!(
        !Arc::ptr_eq(&a, &b),
        "different workspaces must get isolated bindings"
    );

    let a_again = for_workspace(dir_a.path(), &cfg).expect("re-resolve workspace A");
    assert!(
        Arc::ptr_eq(&a, &a_again),
        "same workspace must reuse the cached binding"
    );
}

#[test]
fn same_workspace_with_changed_config_binds_fresh() {
    // `CoreContext::rebind_workspace` treats "same workspace, changed
    // [subsystems.memory]" as a real rebind (a changed driver/hooks/trust all
    // feed `build`). The cache must key on the config as well as the path, or
    // a changed config for an already-bound workspace would keep serving the
    // previous driver until process restart.
    let dir = tempfile::tempdir().unwrap();
    let default = MemorySubsystemConfig::default();
    let null = MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };

    let tiny = for_workspace(dir.path(), &default).expect("bind tinycortex");
    assert_eq!(tiny.driver_id(), "tinycortex");

    // Same (workspace, config) pair reuses the cached binding...
    let tiny_again = for_workspace(dir.path(), &default).expect("re-bind tinycortex");
    assert!(
        Arc::ptr_eq(&tiny, &tiny_again),
        "unchanged config must reuse the cached binding"
    );

    // ...but a changed config for the SAME workspace must bind fresh.
    let null_binding = for_workspace(dir.path(), &null).expect("bind null");
    assert!(
        !Arc::ptr_eq(&tiny, &null_binding),
        "changed config must bind fresh, not serve the stale tinycortex driver"
    );
    assert_eq!(null_binding.driver_id(), "null");

    // Reverting to the original config still resolves its own binding. This is
    // the transient-mismatch half: a stale (workspace, config) pairing never
    // shadows the correct pair, so it cannot permanently pin a workspace to the
    // wrong driver (the atomicity concern in the login/logout rebind).
    let tiny_reverted = for_workspace(dir.path(), &default).expect("re-bind tinycortex");
    assert!(
        Arc::ptr_eq(&tiny, &tiny_reverted),
        "returning to the original config must serve the original binding"
    );
}

#[test]
fn embedded_class_binds_the_embedded_driver_not_null() {
    // Plain `#[test]`: no tokio runtime. Binding must stay synchronous and
    // I/O-free, which is why the embedded driver resolves its client lazily.
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("never-created");
    let binding =
        for_workspace(&workspace, &MemorySubsystemConfig::default()).expect("default bind");

    assert_eq!(binding.driver_id(), "tinycortex");
    assert_eq!(binding.class(), DriverClass::Embedded);
    assert!(binding.fallback().is_none());
    assert_ne!(binding.unguarded_provider().driver_id(), NULL_DRIVER_ID);
    assert!(binding.capabilities().contains(Capability::Core));
    assert!(binding.capabilities().validate().is_ok());
    assert!(
        !workspace.exists(),
        "binding must not touch the workspace on disk"
    );
}

#[test]
fn embedded_binding_advertises_every_family() {
    // Widened once per M3 step; M3d is the last one. The interesting assertion
    // is the second: a *bound* context and an *unbound* one now agree, which
    // they did not for the whole of M2/M3a-c.
    let dir = tempfile::tempdir().unwrap();
    let binding =
        for_workspace(dir.path(), &MemorySubsystemConfig::default()).expect("default bind");
    let advertised = binding.capabilities();

    assert!(advertised.contains_all(Capabilities::mandatory()));
    for family in Capability::ALL {
        assert!(advertised.contains(family), "{family} must be advertised");
    }
    assert_eq!(advertised, Capabilities::all());
    assert_eq!(advertised, unbound_default_capabilities());
}

#[test]
fn null_driver_config_still_binds_the_null_provider() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };
    let binding = for_workspace(dir.path(), &cfg).expect("null bind");
    assert_eq!(binding.driver_id(), NULL_DRIVER_ID);
    assert_eq!(binding.class(), DriverClass::Null);
    assert_eq!(binding.unguarded_provider().driver_id(), NULL_DRIVER_ID);
    assert!(
        binding.fallback().is_none(),
        "an explicitly requested null driver is not a fallback"
    );
}

#[test]
fn refused_driver_falls_back_to_the_null_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let binding =
        for_workspace(dir.path(), &external_driver_cfg("untrusted")).expect("bind falls back");
    assert_eq!(binding.driver_id(), "null");
    assert_eq!(binding.class(), DriverClass::Null);
    let fallback = binding.fallback().expect("fallback provenance recorded");
    assert_eq!(fallback.configured_driver, "supermemory");
}

#[test]
fn fallback_binding_advertises_only_mandatory_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let binding =
        for_workspace(dir.path(), &external_driver_cfg("untrusted")).expect("bind falls back");
    assert_eq!(binding.capabilities(), Capabilities::mandatory());
    // Even the fallback must be a *legal* bind: the mandatory three are present.
    assert!(binding.capabilities().validate().is_ok());
    assert!(!binding.capabilities().contains(Capability::Tree));
}

#[test]
fn unbound_default_is_the_full_capability_set() {
    let all = unbound_default_capabilities();
    assert_eq!(all, Capabilities::all());
    assert_eq!(all.len(), Capability::ALL.len());
}

#[test]
fn bound_driver_view_carries_class_capabilities_and_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let binding =
        for_workspace(dir.path(), &external_driver_cfg("untrusted")).expect("bind falls back");
    let bound = binding.to_bound_driver();
    assert_eq!(bound.slot, SubsystemSlot::Memory);
    assert_eq!(bound.id, "null");
    assert_eq!(bound.class, DriverClass::Null);
    assert_eq!(bound.contract_version, CONTRACT_VERSION);
    assert_eq!(bound.fell_back_from.as_deref(), Some("supermemory"));
    assert!(bound.is_fallback());
    // The generic view carries the same families as opaque strings.
    assert!(bound.capabilities.contains("core"));
    assert!(!bound.capabilities.contains("tree"));
    assert_eq!(bound.capabilities.len(), binding.capabilities().len());
}

#[test]
fn health_converts_as_a_total_three_arm_match() {
    assert_eq!(to_driver_health(MemoryHealth::Ready), DriverHealth::Ready);
    assert_eq!(
        to_driver_health(MemoryHealth::degraded("reindexing")),
        DriverHealth::degraded("reindexing")
    );
    assert_eq!(
        to_driver_health(MemoryHealth::down("refused")),
        DriverHealth::down("refused")
    );
}

// ---- "capabilities asked once" ------------------------------------------
//
// The contract's `MemoryProvider::capabilities` doc says the kernel asks once
// at bind time and caches. Everything downstream (RPC registration, tool
// emission) is filtered from that cached answer, so a second ask would let the
// live surface and the advertised surface drift apart.

struct CountingProvider {
    inner: NullMemoryProvider,
    calls: AtomicUsize,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            inner: NullMemoryProvider::new(),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl MemoryCore for CountingProvider {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        self.inner
            .store(namespace, key, content, category, session_id, taint)
            .await
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.inner.get(namespace, key).await
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.inner.forget(namespace, key).await
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.list(namespace, category, session_id).await
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.inner.namespaces().await
    }
}

#[async_trait]
impl MemoryRecall for CountingProvider {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.recall(query, limit, opts, scope).await
    }
}

#[async_trait]
impl MemoryPortability for CountingProvider {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.inner.export_page(cursor, limit).await
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.inner.import_records(records).await
    }
}

#[async_trait]
impl MemoryProvider for CountingProvider {
    fn driver_id(&self) -> &str {
        "counting"
    }

    fn capabilities(&self) -> Capabilities {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Capabilities::all()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }
}

#[test]
fn capabilities_are_asked_exactly_once_per_bind() {
    let provider = Arc::new(CountingProvider::new());
    let binding = bind_provider_for_test(provider.clone(), DriverClass::Embedded);

    for _ in 0..5 {
        assert_eq!(binding.capabilities(), Capabilities::all());
    }
    assert_eq!(binding.driver_id(), "counting");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "capabilities() must be asked exactly once, at bind time"
    );
}

// ---------------------------------------------------------------------------
// Built-in ids are pinned to their class
// ---------------------------------------------------------------------------
//
// A per-driver table may confirm a built-in id's class but never override it.
// Without that rule `driver = "null"` plus `class = "embedded"` builds the real
// engine and persists memory under the id documented as `/dev/null`, and the
// inverse labels a store-nothing provider `tinycortex`.

fn cfg_with_class(driver: &str, class: &str) -> MemorySubsystemConfig {
    let mut cfg = MemorySubsystemConfig {
        driver: driver.into(),
        ..Default::default()
    };
    cfg.drivers.insert(
        driver.into(),
        MemoryDriverConfig {
            class: Some(class.into()),
            ..Default::default()
        },
    );
    cfg
}

#[test]
fn admit_refuses_an_embedded_class_override_on_the_null_driver() {
    let refusal = admit(&cfg_with_class("null", "embedded"))
        .expect_err("null must not be re-classed as embedded");
    assert_eq!(refusal.configured_driver, "null");
    assert!(
        refusal.reason.contains("built in"),
        "refusal must say the id is built in: {}",
        refusal.reason
    );
}

#[test]
fn admit_refuses_a_null_class_override_on_the_embedded_driver() {
    let refusal = admit(&cfg_with_class(EMBEDDED_DRIVER_ID, "null"))
        .expect_err("tinycortex must not be re-classed as null");
    assert_eq!(refusal.configured_driver, EMBEDDED_DRIVER_ID);
    assert!(
        refusal.reason.contains("built in"),
        "refusal must say the id is built in: {}",
        refusal.reason
    );
}

#[test]
fn admit_accepts_a_class_line_that_agrees_with_the_built_in_id() {
    // Redundant, but not a mistake: confirming the real class is allowed.
    let (id, class) = admit(&cfg_with_class("null", "null")).expect("agreeing class admits");
    assert_eq!(id, "null");
    assert_eq!(class, DriverClass::Null);

    let (id, class) =
        admit(&cfg_with_class(EMBEDDED_DRIVER_ID, "embedded")).expect("agreeing class admits");
    assert_eq!(id, EMBEDDED_DRIVER_ID);
    assert_eq!(class, DriverClass::Embedded);
}

#[test]
fn a_null_class_override_cannot_smuggle_the_embedded_engine_into_the_binding() {
    // The end-to-end shape of the refusal: `build` must not hand back an
    // embedded provider for `driver = "null"`.
    let dir = tempfile::tempdir().unwrap();
    let binding = for_workspace(dir.path(), &cfg_with_class("null", "embedded")).expect("binds");

    assert_eq!(binding.class(), DriverClass::Null);
    assert_eq!(binding.driver_id(), NULL_DRIVER_ID);
    assert!(
        binding.fallback().is_some(),
        "a refused class override must be recorded as a fallback"
    );
}

// ---------------------------------------------------------------------------
// `disables_memory` — deliberate null only
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_null_driver_disables_memory() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };
    let binding = for_workspace(dir.path(), &cfg).expect("binds");

    assert!(binding.fallback().is_none(), "this is not a fallback");
    assert!(
        binding.disables_memory(),
        "an operator who bound /dev/null asked for the surface to be gone"
    );
}

#[test]
fn a_fallback_to_null_does_not_disable_memory() {
    // A misconfiguration must be loud, not silently memory-less: the fallback
    // is reported in status and the surface stays present.
    let dir = tempfile::tempdir().unwrap();
    let binding = for_workspace(dir.path(), &external_driver_cfg("untrusted")).expect("binds");

    assert_eq!(binding.class(), DriverClass::Null);
    assert!(binding.fallback().is_some(), "this IS a fallback");
    assert!(!binding.disables_memory());
}

#[test]
fn the_embedded_driver_never_disables_memory() {
    let dir = tempfile::tempdir().unwrap();
    let binding = for_workspace(dir.path(), &MemorySubsystemConfig::default()).expect("binds");
    assert!(!binding.disables_memory());
}
