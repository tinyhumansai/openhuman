use super::*;
use chrono::Utc;
use serde_json::json;
use tempfile::TempDir;
// `DocumentInput` (and its `ChatBatch` / `EmailThread` siblings, which
// `rpc_tests_part_01_tests` reaches the same way) now arrive through the
// `use super::*` above: they are defined in `rpc_part_01.rs` rather than
// imported from the engine crate. Naming the engine's copy here would compile
// and then fail on the first call into `document_item`, since the two are
// distinct types with identical fields.
use tinymemory_api::chunks::SourceKind;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// Bind a driver reporting fixed diagnostics as `cfg`'s memory driver.
///
/// See `binding::FixedDiagnostics` for why the status handlers need one:
/// they read through the contract now, and the real driver is a compiled
/// module that a unit test cannot load.
fn bind_diagnostics(
    cfg: &Config,
    store: crate::openhuman::memory::api::provider::types::StoreStats,
    queue: crate::openhuman::memory::api::provider::types::QueueStats,
) {
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        store,
        queue,
    );
}

fn sample_document(title: &str, body: &str) -> DocumentInput {
    DocumentInput {
        provider: "notion".into(),
        title: title.into(),
        body: body.into(),
        modified_at: Utc::now(),
        source_ref: Some("notion://page/launch".into()),
    }
}

/// #5324: `queue_idle_ms` measures idle time, not backlog depth. Pins the
/// two shapes that must NOT be reported as stalled, both of which a
/// backlog-age metric would have flagged — and both of which describe the
/// heavy users this issue is about.
/// One queue snapshot, spelled out.
///
/// These read as SQL fixtures before the driver owned the query. What
/// `queue_idle_ms` decides has never depended on the rows, only on the
/// three numbers below, so the tests say those directly now. Which rows
/// produce which numbers is the driver's rule and is pinned in the
/// driver's own suite — notably that deferred work counts as `ready`
/// without becoming `eligible_now`, the distinction the third case here
/// relies on.
fn queue(
    eligible_now: u64,
    last_completed_ms: Option<i64>,
    oldest_eligible_ms: Option<i64>,
) -> crate::openhuman::memory::api::provider::types::QueueStats {
    crate::openhuman::memory::api::provider::types::QueueStats {
        eligible_now,
        last_completed_ms,
        oldest_eligible_ms,
        ..Default::default()
    }
}

/// One failure as the driver reports it.
///
/// These used to plant rows and read the answer back through a `SELECT`.
/// Which rows the driver reports is the driver's rule and is pinned in the
/// driver's own suite; what the host decides to *do* with a reported
/// failure is the rule below, and it depends on nothing but these three
/// values.
fn reported_failure(
    reason: &str,
    failed_at_ms: Option<i64>,
    last_success_ms: Option<i64>,
) -> crate::openhuman::memory::api::provider::types::QueueFailure {
    crate::openhuman::memory::api::provider::types::QueueFailure {
        reason: reason.to_string(),
        class: Some("unrecoverable".to_string()),
        completed_at_ms: failed_at_ms,
        last_success_ms,
    }
}

#[path = "rpc_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "rpc_tests_part_02_tests.rs"]
mod part_02_tests;
