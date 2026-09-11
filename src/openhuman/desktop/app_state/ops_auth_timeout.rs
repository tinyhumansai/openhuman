// How long the current-user refresh may take, and how long a failed run of
// them suppresses the next attempt.
//
// Split out of `ops_part_01.rs` because the two values are one decision: the
// backoff step is *derived* from the fetch timeout so that widening the
// timeout cannot re-open #5624's poll treadmill. Keeping them adjacent is
// what makes that relationship reviewable.
//
// `include!`d into `ops.rs`, so everything here shares that module's scope.

/// Wall-clock budget for one `auth_get_me` refresh when nothing overrides it.
pub const DEFAULT_AUTH_FETCH_TIMEOUT_SECS: u64 = 5;
/// Smallest accepted override. Anything shorter would time out a healthy
/// backend on a merely slow link on every poll — the #5624 treadmill inverted.
pub const MIN_AUTH_FETCH_TIMEOUT_SECS: u64 = 2;
/// Largest accepted override.
///
/// The ceiling exists because this budget is spent *inside* the snapshot RPC,
/// concurrently with [`RUNTIME_SNAPSHOT_TIMEOUT`], and the frontend gives that
/// RPC 30s total. `12 + 10 = 22` leaves headroom; a larger value would let an
/// operator turn a slow backend into a failed snapshot call.
pub const MAX_AUTH_FETCH_TIMEOUT_SECS: u64 = 12;
/// Operator override for [`auth_fetch_timeout`]. A missing, non-numeric or
/// out-of-range value leaves the default in place (and is logged once).
pub const AUTH_FETCH_TIMEOUT_ENV_VAR: &str = "OPENHUMAN_AUTH_FETCH_TIMEOUT_SECS";

/// Floor under the first backoff step, independent of the fetch timeout.
///
/// Must stay above [`CURRENT_USER_REFRESH_TTL`], which governs how soon a poll
/// asks for a live fetch at all.
const CURRENT_USER_BACKOFF_BASE_FLOOR: Duration = Duration::from_secs(10);

/// Parse a raw override into a bounded timeout in seconds.
///
/// Pure and global-free so the clamp can be tested without touching the process
/// environment. `None`, unparseable input, and out-of-range values all fall back
/// to [`DEFAULT_AUTH_FETCH_TIMEOUT_SECS`].
pub fn parse_auth_fetch_timeout_secs(raw: Option<&str>) -> u64 {
    auth_fetch_timeout_override(raw).unwrap_or(DEFAULT_AUTH_FETCH_TIMEOUT_SECS)
}

/// The override if `raw` names one inside the accepted range, else `None`.
///
/// Split from [`parse_auth_fetch_timeout_secs`] so the resolver can tell
/// "accepted" from "fell back" without re-deriving it by comparing strings —
/// which would report a valid `05` as ignored.
fn auth_fetch_timeout_override(raw: Option<&str>) -> Option<u64> {
    raw.map(str::trim)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| (MIN_AUTH_FETCH_TIMEOUT_SECS..=MAX_AUTH_FETCH_TIMEOUT_SECS).contains(n))
}

/// The effective `auth_get_me` timeout, resolved once per process.
///
/// Read through a function rather than held as a `const` so #5930's "5s may be
/// too tight" has an answer that does not need a rebuild. Every caller — the
/// `tokio::time::timeout` wrapper, the timeout log line, and the recorded
/// timeout error's message — reads the same value, so they cannot drift.
fn auth_fetch_timeout() -> Duration {
    static RESOLVED: Lazy<Duration> = Lazy::new(|| {
        let raw = std::env::var(AUTH_FETCH_TIMEOUT_ENV_VAR).ok();
        match (raw.as_deref(), auth_fetch_timeout_override(raw.as_deref())) {
            (Some(raw), None) => warn!(
                "{LOG_PREFIX} ignoring {AUTH_FETCH_TIMEOUT_ENV_VAR}={raw:?}: not an integer in \
                 {MIN_AUTH_FETCH_TIMEOUT_SECS}..={MAX_AUTH_FETCH_TIMEOUT_SECS}; \
                 using {DEFAULT_AUTH_FETCH_TIMEOUT_SECS}s"
            ),
            (Some(_), Some(secs)) => debug!(
                "{LOG_PREFIX} auth fetch timeout overridden to {secs}s by {AUTH_FETCH_TIMEOUT_ENV_VAR}"
            ),
            (None, _) => {}
        }
        Duration::from_secs(parse_auth_fetch_timeout_secs(raw.as_deref()))
    });
    *RESOLVED
}

/// First backoff step after the backend fails to answer `auth_get_me`.
///
/// Derived from `fetch_timeout` rather than fixed, because the property that
/// actually stops the treadmill is *relational*: the step must outlast both the
/// fetch timeout and the frontend's ~5s `app_state_snapshot` poll, or the next
/// poll finds the window already expired and pays the full timeout again
/// (#5624 — 51 timeouts in one session, ~5s each). Making the timeout
/// configurable (#5930) without deriving this would let an operator re-open
/// that bug by widening the timeout past a fixed 10s step.
fn current_user_backoff_base_for(fetch_timeout: Duration) -> Duration {
    CURRENT_USER_BACKOFF_BASE_FLOOR.max(fetch_timeout.saturating_mul(2))
}

/// [`current_user_backoff_base_for`] applied to the effective timeout.
fn current_user_backoff_base() -> Duration {
    current_user_backoff_base_for(auth_fetch_timeout())
}
