use super::*;

/// The frontend's `CODING_SESSION_BATCH_MAX` (`app/src/services/memorySourcesService.ts`).
/// Mirrored here so the cross-wire invariant below is checkable Rust-side; the
/// two must move together.
const CLIENT_BATCH_MAX: usize = 5;
/// The frontend's `PER_CALL_TIMEOUT_MAX_MS` (`app/src/services/coreRpcClient.ts`),
/// in seconds — the ceiling the client can actually wait for.
const CLIENT_HARD_CAP_SECS: u64 = 600;
/// The frontend's `CODING_SESSION_RPC_GRACE_MS`, in seconds.
const CLIENT_GRACE_SECS: u64 = 15;

/// The formula is `min(120 + N * 90, 600)` seconds.
#[test]
fn budget_scales_per_session_for_multiple_windows() {
    // Zero sessions → base overhead only.
    assert_eq!(ingest_budget(0).as_secs(), 120);
    // One session carries a full per-session (multi-window) allowance.
    assert_eq!(ingest_budget(1).as_secs(), 120 + 90);
    // The UI batch (5 sessions) gets 570s — sized so a pass fits under the
    // 600s reachable ceiling while a 15-session backlog drains across passes.
    assert_eq!(ingest_budget(CLIENT_BATCH_MAX).as_secs(), 120 + 5 * 90);
}

/// The budget is hard-capped at the reachable ceiling (600s), so an untrusted
/// `max_sessions` cannot pin the blocking worker beyond it — and cannot
/// overflow.
#[test]
fn budget_is_capped_at_the_reachable_ceiling() {
    assert_eq!(ingest_budget(1_000).as_secs(), CLIENT_HARD_CAP_SECS);
    // Anything at or above the cap yields exactly the ceiling, no overflow.
    assert_eq!(ingest_budget(usize::MAX).as_secs(), CLIENT_HARD_CAP_SECS);
    assert_eq!(ingest_budget(5_000).as_secs(), CLIENT_HARD_CAP_SECS);
}

/// The invariant #5509 actually needs, pinned across the wire: for the UI's
/// batch size the server budget must (a) stay under the client's hard cap so
/// the pass is reachable, and (b) be the *tighter* of the two — i.e. below the
/// client's own timeout for the same batch — so the server returns a clean
/// structured timeout before the client's fetch aborts. This is the guard
/// that would have caught the server-only fix moving the ceiling from 570s to
/// an unreachable 1920s.
#[test]
fn server_budget_is_reachable_and_tighter_than_the_client() {
    let server = ingest_budget(CLIENT_BATCH_MAX).as_secs();
    let client = 120 + (CLIENT_BATCH_MAX as u64) * 90 + CLIENT_GRACE_SECS;
    assert!(
        server <= CLIENT_HARD_CAP_SECS,
        "server budget {server}s must be reachable (<= {CLIENT_HARD_CAP_SECS}s client cap)"
    );
    assert!(
        client <= CLIENT_HARD_CAP_SECS,
        "client budget {client}s must stay under its own {CLIENT_HARD_CAP_SECS}s clamp"
    );
    assert!(
        server < client,
        "server budget {server}s must fire before the client's {client}s abort"
    );
}
