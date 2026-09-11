//! Sibling tests for the admin read-RPCs, covering what moved onto the memory
//! contract in #5560.
//!
//! The three handlers here are the destructive ones, so what is worth pinning
//! host-side is not that a delete deleted — that is the driver's, and the
//! conformance suite proves it against a real store — but **how this host
//! answers when the bound driver cannot serve the family at all**. On a read,
//! a missing family degrades to an honest empty answer; on a wipe or a delete
//! it must not, because a caller reading "nothing was removed" concludes the
//! content was already gone. Each refusal below is that distinction.

use super::{clear_composio_sync_state, delete_source_rpc, wipe_all_rpc};
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::binding;
use std::sync::Arc;
use tempfile::TempDir;
use tinymemory_api::null::NullMemoryProvider;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Keep any persistence these handlers trigger inside the disposable
    // workspace, exactly as `read_rpc_tests::test_config` does.
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// Bind the placeholder driver, which advertises the mandatory three families
/// and returns `None` from every optional `as_*` accessor. That absence is the
/// whole point of the driver, and it is what these tests are about.
fn install_null_driver(cfg: &Config) {
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
    );
}

/// A wipe a driver cannot perform is an error, never a `rows_deleted: 0`
/// success — the caller's next act is telling a user their memory is gone.
/// This mirrors the contract's own choice to default `purge_all` to
/// `Unsupported` while the rest of its family defaults to an empty result.
#[tokio::test]
async fn wipe_all_refuses_a_driver_that_does_not_serve_maintenance() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let err = wipe_all_rpc(&cfg)
        .await
        .expect_err("a driver with no Maintenance family cannot report a completed wipe");
    assert!(
        err.contains("does not serve Maintenance"),
        "the refusal must name the missing family: {err}"
    );
    assert!(
        err.contains("null"),
        "the refusal must name the driver that refused: {err}"
    );
}

/// The opposite call in the same handler, and deliberately not a refusal.
///
/// This replaced a `path.exists()` check that skipped silently, and the number
/// it returned then is the number it returns now: a driver with no key/value
/// tier has nothing of this host's to clear, so zero is a true statement about
/// it rather than a fault a caller can act on.
#[tokio::test]
async fn clear_composio_sync_state_reports_zero_without_a_graph_family() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    assert_eq!(
        clear_composio_sync_state(&cfg)
            .await
            .expect("a driver with no Graph family is not an error"),
        0
    );
}

/// Same reasoning as the wipe: on a delete, a zero the caller reads as
/// "already gone" is worse than a refusal.
#[tokio::test]
async fn delete_source_refuses_a_driver_that_does_not_serve_sources() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let err = delete_source_rpc(&cfg, "notion:page-a".to_string())
        .await
        .expect_err("a driver with no Sources family cannot report a completed delete");
    assert!(
        err.contains("does not serve Sources"),
        "the refusal must name the missing family: {err}"
    );
}

/// Argument validation stays ahead of the driver lookup, so a blank id is a
/// caller error naming itself rather than a round trip that deletes nothing.
/// No driver is installed here on purpose — reaching one would be the failure.
#[tokio::test]
async fn delete_source_rejects_a_blank_source_id_before_touching_a_driver() {
    let (_tmp, cfg) = test_config();

    let err = delete_source_rpc(&cfg, "   ".to_string())
        .await
        .expect_err("a blank source id is a caller error");
    assert!(
        err.contains("must be a non-empty string"),
        "unexpected error: {err}"
    );
}

/// openhuman#6012. Same distinction the wipes above turn on: a backfill the
/// driver cannot perform must not read as `scanned: 0` success. A caller seeing
/// that concludes their stored records are already treed and stops looking —
/// which is exactly the wrong conclusion, because "invisible memories" is the
/// symptom this RPC exists to clear.
#[tokio::test]
async fn backfill_connector_trees_refuses_a_driver_that_serves_no_maintenance() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let err = super::backfill_connector_trees_rpc(&cfg, None, true)
        .await
        .expect_err("a driver with no Maintenance family cannot re-file anything");
    assert!(
        err.contains("does not serve Maintenance"),
        "the refusal must name the family: {err}"
    );
    assert!(
        err.contains(tinymemory_api::null::NULL_DRIVER_ID),
        "and the driver, so an operator can tell which one refused: {err}"
    );
}

/// `executed` reports the **mode**, not the effect — and every counter the
/// driver reports reaches the caller unchanged.
///
/// The counters are deliberately distinct and non-zero. An all-zero fixture
/// would pass just as happily if this RPC dropped the driver's numbers or
/// replaced them with zeros, which is the whole class of bug the mapping can
/// have (review finding). Distinct values also catch a transposition — two
/// fields swapped is invisible when both are 0.
#[tokio::test]
async fn backfill_connector_trees_reports_the_mode_and_passes_the_counters_through() {
    use crate::openhuman::memory::api::provider::types::BackfillTreesOutcome;

    let (_tmp, cfg) = test_config();
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(
            binding::FixedDiagnostics::new(Default::default(), Default::default())
                .backfilling_trees(BackfillTreesOutcome {
                    scanned: 41,
                    ingested: 17,
                    already_present: 23,
                    skipped: 5,
                    more_pending: true,
                    notes: vec!["skill-gmail: skipped".to_string()],
                }),
        ) as Arc<dyn MemoryProvider>,
    );

    let preview = super::backfill_connector_trees_rpc(&cfg, None, true)
        .await
        .expect("a dry run is answerable")
        .value;
    assert!(!preview.executed, "a dry run is the preview mode");

    let real = super::backfill_connector_trees_rpc(&cfg, Some(10), false)
        .await
        .expect("an executing pass is answerable")
        .value;
    assert!(
        real.executed,
        "a real pass reports the mode even when it finds nothing left to do"
    );
    assert_eq!(real.scanned, 41, "scanned must come from the driver");
    assert_eq!(real.ingested, 17, "ingested must come from the driver");
    assert_eq!(
        real.already_present, 23,
        "already_present must come from the driver"
    );
    assert_eq!(real.skipped, 5, "skipped must come from the driver");
    assert!(
        real.more_pending,
        "more_pending must survive; a pass that stopped on its limit must not read as complete"
    );
    assert_eq!(
        real.notes,
        vec!["skill-gmail: skipped".to_string()],
        "the skip reasons are what tell an operator which accounts were left alone"
    );
}
