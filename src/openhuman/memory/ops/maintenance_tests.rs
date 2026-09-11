//! Tests for [`super`] — upkeep asked of the bound driver.
//!
//! A sibling file rather than an inline module: the memory-guard bypass
//! scanner does not brace-track `#[cfg(test)]` blocks, so a test double built
//! from `NullMemoryProvider` inside one reads as a production bypass. Sibling
//! `*_tests.rs` files are excluded by name, which is also this repo's stated
//! convention for tests that outgrow a few lines.

use super::*;

fn test_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

/// A driver with no `Maintenance` reports no work rather than refusing.
///
/// This is the shape every call site depends on: each one runs after a save
/// that already succeeded, so an error here would have nowhere to go. The
/// engine call this replaced swallowed its own failures, and the contract
/// keeps the same promise for a driver that simply has no re-embedding to
/// do.
#[tokio::test]
async fn a_driver_without_maintenance_enqueues_nothing_and_does_not_error() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        std::sync::Arc::new(tinymemory_api::null::NullMemoryProvider::new()),
    );

    assert_eq!(
        reembed(&cfg)
            .await
            .expect("a missing family is not an error"),
        0,
        "no driver to enqueue work means no work enqueued, which is true \
         rather than a refusal"
    );
}

/// The driver's own count reaches the caller.
///
/// `reembed` is coverage-gated at the driver — it enqueues only what is
/// uncovered — so the number is the only way a caller can tell "nothing to
/// do" from "did something", and it has to be the driver's number rather
/// than one this layer invents.
#[tokio::test]
async fn the_drivers_enqueued_count_reaches_the_caller() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Default::default(),
        Default::default(),
    );

    // `FixedDiagnostics` answers the maintenance operations with an empty
    // report, so this pins the plumbing: a driver that serves the family is
    // asked, and its `changed` is what comes back.
    assert_eq!(reembed(&cfg).await.expect("driver serves Maintenance"), 0);
}
