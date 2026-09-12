use super::*;

#[test]
fn response_keeps_top_level_statuses_array() {
    let value = serde_json::to_value(StatusListResponse {
        statuses: Vec::new(),
    })
    .unwrap();
    assert!(value
        .get("statuses")
        .is_some_and(serde_json::Value::is_array));
}

// ── the degradation contract ────────────────────────────────────────────────
//
// `status_list_rpc` answers `Ok` with an empty list on **every** failure path:
// the binding failing to resolve, the bound driver not serving `SourceSync`,
// and the driver's own call returning an error. Its comment says this is
// inherited behaviour kept deliberately — "this surface renders a status table,
// and every caller of it today treats 'no rows' as 'nothing syncing'" — and
// that tightening it belongs with whoever owns that screen.
//
// Nothing asserted any of the three. They were covered only by
// `tests/raw_coverage/memory_threads_raw_coverage_e2e.rs`, a 4,800-line target
// that drove forty subjects through an in-process engine and went with it
// (#6161). A later "tighten this into an RPC error" would turn a polled status
// table into a visible failure, and no test would have objected (#6172).
//
// Two of the three are asserted below. The first — the binding itself failing
// to resolve — is deliberately not, because it cannot be reached from a unit
// test without damaging the rest of the binary. `binding::for_subtree` returns
// `Err` on exactly two conditions: the process-wide `BINDINGS` lock being
// poisoned, and the process-wide exit gate being raised. `memory::exit`'s own
// doc gives the reason that gate is a type with one production instance rather
// than a bare static — "a process-wide refusal to bind memory would reach every
// other test in the same process" — so a test that raised it would fail every
// test that binds memory after it. The arm is a two-line `warn` and `Vec::new()`
// in `rpc.rs`; what would have to change for it to start propagating is the
// `match` around it, which the two cases below already hold in place.

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::binding;
use std::sync::Arc;
use tempfile::TempDir;
use tinymemory_api::null::NullMemoryProvider;

fn degradation_config() -> (TempDir, Config) {
    let tmp = TempDir::new().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.config_path = tmp.path().join("config.toml");
    (tmp, cfg)
}

/// A driver that serves `SourceSync` and fails the call — the third path, which
/// neither the null driver nor a healthy one can produce.
#[derive(Debug, Default)]
struct FailingSourceSync {
    /// Proves the handler reached this driver. An empty list is also what an
    /// *unreached* driver yields, so the assertion below would pass without it
    /// even if the binding never resolved here.
    calls: std::sync::Mutex<usize>,
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemorySourceSync for FailingSourceSync {
    async fn sync_statuses(
        &self,
    ) -> Result<Vec<tinymemory_api::provider::SourceSyncStatus>, tinymemory_api::error::MemoryError>
    {
        *self.calls.lock().expect("calls lock") += 1;
        Err(tinymemory_api::error::MemoryError::Backend(
            "the backend is unreachable".into(),
        ))
    }

    async fn source_sync_state(
        &self,
        _toolkit: &str,
        _connection_id: &str,
    ) -> Result<Option<tinymemory_api::provider::SourceSyncState>, tinymemory_api::error::MemoryError>
    {
        Ok(None)
    }

    async fn run_connection_sync(
        &self,
        _toolkit: &str,
        _connection_id: &str,
    ) -> Result<tinymemory_api::provider::SyncRunOutcome, tinymemory_api::error::MemoryError> {
        Ok(tinymemory_api::provider::SyncRunOutcome::default())
    }

    async fn sync_audit_log(
        &self,
        _limit: Option<usize>,
    ) -> Result<Vec<tinymemory_api::provider::SyncAuditEntry>, tinymemory_api::error::MemoryError>
    {
        Ok(Vec::new())
    }

    async fn estimate_sync_cost_usd(
        &self,
        _input_tokens: u64,
        _output_tokens: u64,
    ) -> Result<f64, tinymemory_api::error::MemoryError> {
        Ok(0.0)
    }

    async fn raw_archive_coverage(
        &self,
        _tree_scope: &str,
        _archive_source_id: &str,
    ) -> Result<tinymemory_api::provider::RawArchiveCoverage, tinymemory_api::error::MemoryError>
    {
        Ok(tinymemory_api::provider::RawArchiveCoverage::default())
    }

    async fn rebuild_from_raw_archive(
        &self,
        _tree_scope: &str,
        _archive_source_id: &str,
    ) -> Result<tinymemory_api::provider::RawRebuildOutcome, tinymemory_api::error::MemoryError>
    {
        Ok(tinymemory_api::provider::RawRebuildOutcome::default())
    }
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryCore for FailingSourceSync {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: tinymemory_api::types::MemoryCategory,
        _session_id: Option<&str>,
        _taint: tinymemory_api::types::MemoryTaint,
    ) -> Result<(), tinymemory_api::error::MemoryError> {
        Ok(())
    }
    async fn get(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> Result<Option<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError>
    {
        Ok(None)
    }
    async fn forget(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> Result<bool, tinymemory_api::error::MemoryError> {
        Ok(false)
    }
    async fn namespaces(
        &self,
    ) -> Result<Vec<tinymemory_api::types::NamespaceSummary>, tinymemory_api::error::MemoryError>
    {
        Ok(Vec::new())
    }
    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&tinymemory_api::types::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryRecall for FailingSourceSync {
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: &tinymemory_api::recall::OwnedRecallOpts,
        _scope: Option<&tinymemory_api::provider::SourceScope>,
    ) -> Result<Vec<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryPortability for FailingSourceSync {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<tinymemory_api::provider::ExportPage, tinymemory_api::error::MemoryError> {
        Ok(tinymemory_api::provider::ExportPage::default())
    }
    async fn import_records(
        &self,
        _records: Vec<tinymemory_api::provider::ExportRecord>,
    ) -> Result<tinymemory_api::provider::ImportOutcome, tinymemory_api::error::MemoryError> {
        Ok(tinymemory_api::provider::ImportOutcome::default())
    }
}

#[async_trait::async_trait]
impl MemoryProvider for FailingSourceSync {
    fn driver_id(&self) -> &str {
        "failing-source-sync-test-driver"
    }
    fn capabilities(&self) -> tinymemory_api::capabilities::Capabilities {
        tinymemory_api::capabilities::Capabilities::mandatory()
    }
    async fn health(&self) -> tinymemory_api::health::MemoryHealth {
        tinymemory_api::health::MemoryHealth::Ready
    }
    fn as_source_sync(&self) -> Option<&dyn tinymemory_api::provider::MemorySourceSync> {
        Some(self)
    }
}

/// Path two: the driver is bound and does not serve `SourceSync`.
#[tokio::test]
async fn a_driver_without_the_family_reports_an_empty_table_rather_than_an_error() {
    let (_tmp, cfg) = degradation_config();
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
    );

    let outcome = status_list_rpc(&cfg)
        .await
        .expect("an unserved family must not surface as an RPC error");
    assert!(
        outcome.value.statuses.is_empty(),
        "the table is empty, not broken"
    );
}

/// Path three: the driver serves the family and the call fails.
///
/// The one a null driver cannot produce, and the one most likely to be
/// "tightened" by someone who has not read the comment — a backend blip is
/// exactly what looks like it deserves an error.
#[tokio::test]
async fn a_driver_error_is_degraded_to_an_empty_table_not_surfaced() {
    let (_tmp, cfg) = degradation_config();
    let driver = Arc::new(FailingSourceSync::default());
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::clone(&driver) as Arc<dyn MemoryProvider>,
    );

    let outcome = status_list_rpc(&cfg)
        .await
        .expect("a driver-side failure must not surface as an RPC error");
    assert!(outcome.value.statuses.is_empty());
    assert_eq!(
        *driver.calls.lock().expect("calls lock"),
        1,
        "the handler must have reached the driver — an empty table from an \
         unreached driver would prove nothing about the degradation"
    );
}
