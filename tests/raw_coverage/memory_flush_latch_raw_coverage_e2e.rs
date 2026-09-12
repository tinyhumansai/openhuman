//! Raw integration coverage for the `flush_source_tree` re-entrancy latch
//! (#5779).
//!
//! `flush_source_tree_rpc` keeps a process-global `ACTIVE` set so a second
//! concurrent call for the same scope is answered "already running" instead of
//! being run twice. The version this replaced removed the scope from that set
//! only after a *successful* flush, so any failing call left its scope latched
//! for the life of the process: the retry a caller would naturally make came
//! back "already running" forever, and that source could never be flushed
//! again without a restart. #5779 made the release an RAII guard that drops on
//! every exit, error paths included.
//!
//! The test drives a failing flush and then retries the same scope. What is
//! asserted is that the retry is *attempted* — it must come back as a failure
//! from the driver lookup again, never as the latch's "already running"
//! success. Nothing here asserts which error the driver gives, only that the
//! second call got as far as asking.

use std::sync::OnceLock;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::read_rpc;
use tempfile::TempDir;

static WORKSPACE: OnceLock<TempDir> = OnceLock::new();

/// A scope string unique to this test.
///
/// `ACTIVE` is a process-global `static`, and every `raw_coverage` suite now
/// shares one test binary, so a scope another test also used would make this
/// one's result depend on run order.
const SCOPE: &str = "memory_flush_latch_raw_coverage_e2e::retry-after-failure";

/// A flush that fails must not latch its scope out of every later attempt.
///
/// Both calls are expected to fail: an integration test has no memory module
/// bound, so the handler cannot reach a driver serving `Tree`. That is exactly
/// the shape the bug needed — a flush that does not reach a successful return.
/// With the pre-#5779 latch the first failure would leave `SCOPE` in `ACTIVE`,
/// and the second call would short-circuit to `Ok` before touching the driver.
#[tokio::test]
async fn a_failed_flush_source_tree_can_be_retried_for_the_same_scope() {
    let workspace = WORKSPACE.get_or_init(|| TempDir::new().expect("workspace tempdir"));
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();

    let first = read_rpc::flush_source_tree_rpc(&config, SCOPE).await;
    assert!(
        first.is_err(),
        "this test needs a flush that does not succeed, so the latch release is \
         exercised on an error path; got {first:?}"
    );

    let second = read_rpc::flush_source_tree_rpc(&config, SCOPE).await;

    // The failure mode being pinned: `Ok` here is the latch answering
    // "already running" for a flush that is not running, which is what the
    // pre-#5779 handler did for every scope whose first flush failed.
    assert!(
        second.is_err(),
        "a retry after a failed flush must reach the driver again, not be \
         short-circuited by the re-entrancy latch; got {second:?}"
    );
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "the retry must fail the same way the first attempt did — a different \
         answer means it took a different path"
    );
}
