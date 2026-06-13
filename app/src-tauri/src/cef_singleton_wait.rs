//! Windows pre-CEF cache-lock wait (Sentry TAURI-RUST-F).
//!
//! The vendored `tauri-runtime-cef` calls `cef::initialize()` and asserts the
//! result equals `1`. When the CEF user-data-dir is still in use by another
//! OpenHuman process, `cef::initialize()` returns `0` and that assert panics
//! with `assertion left == right failed, left: 0, right: 1` — a fatal,
//! actionless crash (Sentry TAURI-RUST-F, ~3.2k events, Windows-only).
//!
//! The existing pre-CEF Win32 named-mutex guard (see `run()` in `lib.rs`)
//! already handles the *concurrent* second-launch case: a second instance
//! sees `ERROR_ALREADY_EXISTS` and exits before touching CEF. What it does
//! NOT cover is the *sequential relaunch race*:
//!
//!   1. Instance A's `run()` returns → the RAII mutex guard drops → the named
//!      mutex is released.
//!   2. A is still tearing down: its CEF browser process (and child
//!      processes) keep the user-data-dir cache lock for a short window.
//!   3. Instance B launches (auto-update relaunch, fast quit+reopen, a
//!      restart flow), acquires the now-free mutex, and calls
//!      `cef::initialize()` while A's cache lock is still held → `0` → panic.
//!
//! This module closes that window the same way macOS does with
//! `process_recovery::reap_stale_openhuman_processes`: before CEF init, wait
//! (bounded) for the prior instance's process(es) to exit, then proceed. If
//! the holder is still alive after the budget, exit cleanly instead of
//! initializing into a locked cache — the panic is *prevented* (we never call
//! `cef::initialize()` against a live lock), not caught/suppressed.
//!
//! Why counting same-exe processes is correct here, despite CEF spawning
//! same-exe subprocesses:
//!   - This runs BEFORE `cef::initialize()`, so *our own* CEF subprocesses do
//!     not exist yet — none of the counted processes are ours.
//!   - It runs AFTER the Win32 mutex guard, which guarantees we are the only
//!     top-level instance past that point. Any other same-exe PIDs therefore
//!     belong to the *dying prior instance* (its browser + CEF children).
//! So "other same-exe instances > 0" is a faithful "prior instance not gone
//! yet" signal, and drops to 0 once the prior instance fully exits.
//!
//! Validation note: this is Windows-only CEF-startup behaviour that cannot be
//! reproduced on a macOS dev host. The decision logic ([`decide`],
//! [`backoff_delay`]) is a pure function covered by unit tests that run on any
//! host; the Win32 process enumeration and the wait loop are exercised via a
//! Windows runner / manual repro.

use std::time::Duration;

/// Maximum time to wait for a dying prior instance to release the CEF cache
/// before giving up and exiting cleanly. Kept short so a genuinely stuck
/// holder cannot hang startup indefinitely — the common relaunch race
/// resolves well under a second.
const WAIT_BUDGET: Duration = Duration::from_secs(5);

/// First poll backoff; doubles each attempt up to [`BACKOFF_CAP`].
const BACKOFF_BASE: Duration = Duration::from_millis(100);

/// Upper bound on a single backoff sleep so we keep polling responsively as
/// the prior instance winds down.
const BACKOFF_CAP: Duration = Duration::from_millis(500);

/// What the wait loop should do given the current observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitDecision {
    /// No other instance holds the cache — safe to call `cef::initialize()`.
    Proceed,
    /// A prior instance is still alive and we are within the time budget —
    /// sleep and re-check.
    KeepWaiting,
    /// A prior instance is still alive but the budget is exhausted — exit
    /// cleanly rather than initialize into a locked cache (which would panic).
    GiveUp,
}

/// Pure decision for the wait loop. Separated from the Win32 glue so it is
/// unit-testable on any host.
fn decide(other_instances: usize, elapsed: Duration, budget: Duration) -> WaitDecision {
    if other_instances == 0 {
        WaitDecision::Proceed
    } else if elapsed >= budget {
        WaitDecision::GiveUp
    } else {
        WaitDecision::KeepWaiting
    }
}

/// Exponential backoff for poll `attempt` (0-based), capped at [`BACKOFF_CAP`].
/// Pure + saturating so it never overflows on a high attempt count.
fn backoff_delay(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
    BACKOFF_BASE
        .checked_mul(factor)
        .unwrap_or(BACKOFF_CAP)
        .min(BACKOFF_CAP)
}

/// Block (bounded) until no other OpenHuman instance holds the CEF cache, then
/// return so the caller can proceed to `cef::initialize()`. If the holder is
/// still alive after [`WAIT_BUDGET`], forward any pending deep links to it and
/// exit cleanly — never initialize into a locked cache.
#[cfg(windows)]
pub(crate) fn wait_for_cache_release() {
    use std::time::Instant;

    let start = Instant::now();
    let mut attempt: u32 = 0;

    loop {
        let others = win::count_other_app_instances();
        match decide(others, start.elapsed(), WAIT_BUDGET) {
            WaitDecision::Proceed => {
                if attempt > 0 {
                    log::info!(
                        "[cef-singleton-wait] prior instance released CEF cache after {} poll(s) ({} ms); proceeding to CEF init",
                        attempt,
                        start.elapsed().as_millis()
                    );
                }
                return;
            }
            WaitDecision::KeepWaiting => {
                let delay = backoff_delay(attempt);
                log::warn!(
                    "[cef-singleton-wait] {others} prior OpenHuman process(es) still alive (CEF cache lock); waiting {} ms before re-check (elapsed {} ms, TAURI-RUST-F)",
                    delay.as_millis(),
                    start.elapsed().as_millis()
                );
                std::thread::sleep(delay);
                attempt = attempt.saturating_add(1);
            }
            WaitDecision::GiveUp => {
                log::error!(
                    "[cef-singleton-wait] {others} prior OpenHuman process(es) still hold the CEF cache after {} ms budget; exiting cleanly instead of panicking in cef::initialize (TAURI-RUST-F)",
                    WAIT_BUDGET.as_millis()
                );
                // Best-effort: hand any openhuman:// deep links in our argv to
                // the surviving primary before we exit, so an OAuth callback
                // that triggered this relaunch is not dropped. Mirrors the
                // mutex secondary-exit path.
                match crate::deep_link_ipc_windows::try_forward_deep_links() {
                    crate::deep_link_ipc_windows::ForwardResult::Forwarded
                    | crate::deep_link_ipc_windows::ForwardResult::NoUrls => {}
                    crate::deep_link_ipc_windows::ForwardResult::NoPrimary => {
                        log::warn!(
                            "[cef-singleton-wait] had deep-link argv but could not reach a primary pipe before exit"
                        );
                    }
                }
                std::process::exit(0);
            }
        }
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    /// Count running processes whose executable basename matches our own,
    /// excluding the current process. After the pre-CEF mutex guard this
    /// equals the number of straggler processes from a dying prior instance
    /// (its browser + CEF children) — see the module docs.
    ///
    /// Best-effort: on any Win32 error we return 0 (proceed). Refusing to
    /// start because we could not enumerate would be strictly worse than the
    /// rare panic this guard exists to avoid.
    pub(super) fn count_other_app_instances() -> usize {
        let Some(self_exe) = current_exe_name_lower() else {
            log::warn!(
                "[cef-singleton-wait] could not resolve current exe name; skipping prior-instance wait"
            );
            return 0;
        };
        let self_pid = std::process::id();

        // SAFETY: CreateToolhelp32Snapshot returns a handle we close below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        // INVALID_HANDLE_VALUE is -1 as an isize-shaped HANDLE.
        if snapshot.is_null() || snapshot as isize == -1 {
            log::warn!("[cef-singleton-wait] CreateToolhelp32Snapshot failed; skipping wait");
            return 0;
        }

        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut count = 0usize;
        // SAFETY: snapshot is valid; entry.dwSize set per the Win32 contract.
        if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
            loop {
                if entry.th32ProcessID != self_pid && exe_name_matches(&entry.szExeFile, &self_exe)
                {
                    count += 1;
                }
                // SAFETY: snapshot + entry remain valid for the walk.
                if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                    break;
                }
            }
        }

        // SAFETY: snapshot was a valid handle from CreateToolhelp32Snapshot.
        unsafe { CloseHandle(snapshot) };
        count
    }

    fn current_exe_name_lower() -> Option<String> {
        std::env::current_exe()
            .ok()?
            .file_name()?
            .to_str()
            .map(|s| s.to_ascii_lowercase())
    }

    /// Case-insensitive compare of a null-terminated UTF-16 `szExeFile`
    /// against our (already-lowercased) exe basename.
    fn exe_name_matches(sz_exe_file: &[u16], self_exe_lower: &str) -> bool {
        let end = sz_exe_file
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(sz_exe_file.len());
        let name = String::from_utf16_lossy(&sz_exe_file[..end]).to_ascii_lowercase();
        name == self_exe_lower
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_proceeds_when_no_other_instance() {
        assert_eq!(
            decide(0, Duration::from_millis(0), WAIT_BUDGET),
            WaitDecision::Proceed
        );
        // Even past the budget, zero holders means proceed.
        assert_eq!(
            decide(0, WAIT_BUDGET + Duration::from_secs(1), WAIT_BUDGET),
            WaitDecision::Proceed
        );
    }

    #[test]
    fn decide_keeps_waiting_within_budget() {
        assert_eq!(
            decide(1, Duration::from_millis(0), WAIT_BUDGET),
            WaitDecision::KeepWaiting
        );
        assert_eq!(
            decide(3, WAIT_BUDGET - Duration::from_millis(1), WAIT_BUDGET),
            WaitDecision::KeepWaiting
        );
    }

    #[test]
    fn decide_gives_up_at_budget_boundary() {
        // elapsed == budget is the give-up boundary (>=).
        assert_eq!(decide(1, WAIT_BUDGET, WAIT_BUDGET), WaitDecision::GiveUp);
        assert_eq!(
            decide(2, WAIT_BUDGET + Duration::from_secs(1), WAIT_BUDGET),
            WaitDecision::GiveUp
        );
    }

    #[test]
    fn backoff_is_exponential_then_capped() {
        assert_eq!(backoff_delay(0), BACKOFF_BASE); // 100ms
        assert_eq!(backoff_delay(1), Duration::from_millis(200));
        assert_eq!(backoff_delay(2), Duration::from_millis(400));
        // 800ms would exceed the cap → clamped.
        assert_eq!(backoff_delay(3), BACKOFF_CAP); // 500ms
        assert_eq!(backoff_delay(10), BACKOFF_CAP);
    }

    #[test]
    fn backoff_saturates_on_huge_attempt() {
        // Must not panic/overflow on an absurd attempt count.
        assert_eq!(backoff_delay(u32::MAX), BACKOFF_CAP);
    }
}
