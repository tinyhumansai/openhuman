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
use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::api::CONTRACT_VERSION;
use tinymemory_api::null::{NullMemoryProvider, NULL_DRIVER_ID};

use crate::openhuman::memory::api::capabilities::Capability;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::types::{
    ExportPage, ExportRecord, ImportOutcome, SourceScope,
};
use crate::openhuman::memory::api::provider::{MemoryCore, MemoryPortability, MemoryRecall};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};
use async_trait::async_trait;

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

// ---------------------------------------------------------------------------
// Built-in ids are pinned to their class
// ---------------------------------------------------------------------------
//
// A per-driver table may confirm a built-in id's class but never override it.
// Without that rule `driver = "null"` plus `class = "module"` builds the real
// engine and persists memory under the id documented as `/dev/null`, and the
// inverse labels a store-nothing provider `tinymemory`.

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

#[path = "binding_tests_part_01_tests.rs"]
mod part_01_tests;
