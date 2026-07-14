//! macOS/Linux pre-CEF stale CEF-lock-holder reap (issue #4395, follow-up to
//! #3605).
//!
//! ## The gap this closes
//!
//! On Windows a Win32 named mutex early in `run()` forces any *concurrent*
//! peer to `exit(0)` before the pre-CEF reap, so a process still holding the
//! CEF cache lock past that point is provably a wedged prior instance —
//! `process_recovery::reap_stale_openhuman_processes` reaps it safely.
//!
//! macOS and Linux have **no equivalent pre-CEF single-instance guard**: the
//! `#[cfg(windows)]` mutex does not exist there and `tauri_plugin_single_instance`
//! registers later (inside the builder, after CEF init). So a process holding
//! the CEF `SingletonLock` past the [`cef_preflight`] cache-wait budget may be a
//! **healthy running primary** — a live app holds its lock for its whole
//! lifetime — not a wedged one. Reaping it purely because "the lock was held N
//! seconds" could SIGKILL a healthy instance and lose user data. That exact
//! macOS SIGKILL escalation was reverted from PR #3793 (flagged by @oxoxDev +
//! Codex P1), and this module must not reintroduce it.
//!
//! ## The staleness signal (why this is safe)
//!
//! Rather than a time budget, the reap is gated on a **post-update relaunch
//! marker** — the "post-update marker" staleness signal named in #4395. The
//! updater writes the marker ([`write_update_relaunch_marker`]) immediately
//! before `app.restart()`; the freshly relaunched process consumes it here.
//!
//! Only when a *recent* marker is present do we treat a live CEF-lock holder as
//! stale. That is the one situation where a survivor is provably the
//! pre-update instance that should already have exited — never a healthy,
//! separately-launched app (single-instance app; no marker on a normal launch).
//! The marker is consumed (deleted) once per launch and bounded by
//! [`MARKER_MAX_AGE`] so a leaked marker cannot make a much-later launch reap a
//! healthy instance.
//!
//! Additional guards before any kill:
//!   - **Self**: never reap our own pid.
//!   - **Host**: the `SingletonLock` symlink target is `<hostname>-<pid>`; skip
//!     if the holder's host differs from ours (networked/NFS home dir where the
//!     pid belongs to another machine and could collide with an unrelated local
//!     process).
//!   - **Re-validate before SIGKILL**: after SIGTERM + grace, re-read the lock
//!     and confirm the *same* pid still owns it and is still alive. This closes
//!     the PID-reuse window and honors `kill_pid_force`'s contract.
//!
//! This is complementary to `process_recovery::reap_stale_openhuman_processes`
//! (which deliberately *skips* when a live CEF-lock holder exists) and to the
//! Windows-only reap hardening tracked separately in #3900 — no Windows code is
//! touched here.
//!
//! Validation note: the reap glue is `#[cfg(any(macos, linux))]` and cannot be
//! reproduced on an arbitrary dev host. The decision logic ([`decide_reap`],
//! [`marker_is_fresh`], [`marker_path_for`]) is pure and unit-tested on any
//! host; the filesystem/signal glue is exercised on real macOS/Linux hosts.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Grace period between SIGTERM and the re-validated SIGKILL. Mirrors the
/// `TERM_GRACE` used by `process_recovery`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const TERM_GRACE: Duration = Duration::from_millis(500);

/// Bounded grace the reaper gives a live CEF-lock holder to release the lock on
/// its own *before* escalating to SIGTERM. On a normal post-update relaunch the
/// prior runtime is still tearing down and briefly still owns the lock (the
/// common sequential-relaunch race the preflight wait absorbs); only a holder
/// still wedged after this grace is force-reaped, so an ordinary update never
/// terminates a healthy instance (issue #4395 review).
#[cfg(any(target_os = "macos", target_os = "linux"))]
const RELAUNCH_TEARDOWN_GRACE: Duration = Duration::from_secs(2);

/// A relaunch marker older than this is ignored (and cleaned up). Bounds the
/// blast radius of a marker leaked by a process that died before consuming it,
/// so it can never make an unrelated later launch reap a healthy instance.
const MARKER_MAX_AGE: Duration = Duration::from_secs(300);

/// File name of the post-update relaunch marker, written as a sibling of the
/// CEF cache directory (outside the locked `cef/` dir).
const MARKER_FILE_NAME: &str = "openhuman-update-relaunch.marker";

/// What to do about the current CEF `SingletonLock` holder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReapDecision {
    /// Leave the holder alone; the normal `cef_preflight` wait handles it.
    Skip(&'static str),
    /// Holder is provably a stale pre-update instance — reap this (host, pid).
    Reap { host: String, pid: i32 },
}

/// Pure reap decision, separated from the filesystem/signal glue so it is
/// unit-testable on any host.
///
/// `holder` is the *live* `SingletonLock` holder as `(host, pid)`, or `None`
/// when there is no lock / it is stale / the pid is dead. `local_host` is this
/// machine's hostname; `None` (unresolved) fails closed — we cannot prove the
/// holder is same-host, so we skip. `marker_pid` is the pid recorded by a fresh
/// post-update marker (`None` when absent/stale); it must equal the holder pid,
/// so a leaked marker cannot authorize killing an unrelated same-host process.
fn decide_reap(
    holder: Option<(String, i32)>,
    self_pid: i32,
    local_host: Option<&str>,
    marker_pid: Option<i32>,
) -> ReapDecision {
    let Some((host, pid)) = holder else {
        return ReapDecision::Skip("no live CEF SingletonLock holder");
    };
    if pid == self_pid {
        return ReapDecision::Skip("CEF lock held by self");
    }
    // Fail closed on host identity: if we can't resolve our own hostname we
    // cannot prove the holder is same-host, and on a shared/NFS cache path a
    // `<host>-<pid>` can belong to another machine with the same numeric pid.
    let Some(local) = local_host else {
        return ReapDecision::Skip("local hostname unresolved; cannot prove same-host lock holder");
    };
    if local != host {
        return ReapDecision::Skip("CEF lock holder is on a different host");
    }
    // The decisive safety gate: reap only on a recent post-update marker whose
    // recorded pid matches THIS lock holder. Freshness alone is not enough — a
    // leaked fresh marker must never authorize killing whichever same-host
    // process happens to own the lock now.
    match marker_pid {
        None => ReapDecision::Skip("no recent post-update marker; deferring to preflight wait"),
        Some(mpid) if mpid != pid => {
            ReapDecision::Skip("post-update marker pid does not match the live CEF-lock holder")
        }
        Some(_) => ReapDecision::Reap { host, pid },
    }
}

/// Pure freshness predicate: a marker is fresh iff its age is known and within
/// `max_age`. An unknown age (`None`) is treated as not fresh (fail-safe: do
/// not reap on an unreadable timestamp).
fn marker_is_fresh(age: Option<Duration>, max_age: Duration) -> bool {
    matches!(age, Some(a) if a <= max_age)
}

/// Parse the pid recorded in a relaunch-marker body (`pid=<n>`), tolerating a
/// trailing newline and surrounding whitespace. `None` if absent/unparseable —
/// then no reap is authorized (the pid must match the live lock holder).
fn parse_marker_pid(body: &str) -> Option<i32> {
    body.lines()
        .find_map(|line| line.trim().strip_prefix("pid="))
        .and_then(|v| v.trim().parse::<i32>().ok())
}

/// Pure derivation of the marker path from the CEF cache directory. The marker
/// is a sibling of the cache dir (e.g. `.../com.openhuman.app/cef` →
/// `.../com.openhuman.app/openhuman-update-relaunch.marker`) so it lives
/// outside the locked `cef/` directory.
fn marker_path_for(cache_path: &Path) -> PathBuf {
    cache_path.with_file_name(MARKER_FILE_NAME)
}

/// Resolve the CEF cache directory. In production `cef_profile::prepare_process_cache_path`
/// always sets `OPENHUMAN_CEF_CACHE_PATH` before both the updater and this reap
/// run, so env resolution is sufficient and keeps this cross-platform/testable.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn cef_cache_path() -> Option<PathBuf> {
    std::env::var_os("OPENHUMAN_CEF_CACHE_PATH").map(PathBuf::from)
}

/// Write the post-update relaunch marker. Called by the updater immediately
/// before `app.restart()`; the freshly relaunched process consumes it in
/// [`reap_stale_cef_lock_holder`]. Best-effort — a write failure only means the
/// reap falls back to the (safe) preflight wait.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn write_update_relaunch_marker() {
    let Some(cache) = cef_cache_path() else {
        log::warn!(
            "[cef-stale-reap] OPENHUMAN_CEF_CACHE_PATH unset; skipping update-relaunch marker write"
        );
        return;
    };
    let marker = marker_path_for(&cache);
    let body = format!("pid={}\n", std::process::id());
    match std::fs::write(&marker, body) {
        Ok(()) => log::info!(
            "[cef-stale-reap] wrote update-relaunch marker at {} (pid={})",
            marker.display(),
            std::process::id()
        ),
        Err(e) => log::warn!(
            "[cef-stale-reap] failed to write update-relaunch marker at {}: {e}",
            marker.display()
        ),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use std::path::Path;

    use super::{
        cef_cache_path, decide_reap, marker_is_fresh, marker_path_for, parse_marker_pid,
        ReapDecision, MARKER_MAX_AGE, RELAUNCH_TEARDOWN_GRACE, TERM_GRACE,
    };
    use crate::cef_preflight;
    use crate::core_process;
    use crate::process_kill::{kill_pid_force, kill_pid_term};

    /// Pre-CEF reap of a wedged prior instance that still holds the CEF
    /// `SingletonLock` after an update relaunch. Safe by construction: only
    /// acts when a recent post-update marker is present (see module docs).
    /// Best-effort throughout — any failure leaves the holder for the normal
    /// `cef_preflight::wait_for_cache_release` path.
    pub(crate) fn reap_stale_cef_lock_holder() {
        if core_process::reuse_existing_listener_enabled() {
            log::info!(
                "[cef-stale-reap] OPENHUMAN_CORE_REUSE_EXISTING=1; skipping stale CEF-lock reap"
            );
            return;
        }

        let Some(cache) = cef_cache_path() else {
            log::debug!("[cef-stale-reap] OPENHUMAN_CEF_CACHE_PATH unset; nothing to reap");
            return;
        };
        let lock_path = cache.join("SingletonLock");
        let marker_path = marker_path_for(&cache);

        // Consume the marker exactly once per launch (delete regardless of the
        // decision) so a normal launch can never inherit a leftover signal.
        let marker_pid = consume_marker(&marker_path);

        let holder = live_lock_holder(&lock_path);
        let self_pid = std::process::id() as i32;
        let local_host = local_hostname();

        match decide_reap(holder, self_pid, local_host.as_deref(), marker_pid) {
            ReapDecision::Skip(reason) => {
                log::debug!("[cef-stale-reap] not reaping CEF-lock holder: {reason}");
            }
            ReapDecision::Reap { host, pid } => reap_pid(&lock_path, &host, pid),
        }
    }

    /// SIGTERM the stale pid, wait a grace period, then — only if the *same*
    /// pid still owns the lock and is still alive — SIGKILL it. The re-check
    /// closes the PID-reuse window before the force-kill.
    fn reap_pid(lock_path: &Path, host: &str, pid: i32) {
        let pid_u32 = pid as u32;
        // #1 (issue #4395 review): a fresh marker + live holder is the EXPECTED
        // transient on a normal relaunch — the prior runtime is still tearing
        // down. Give that teardown a bounded grace to release the lock before
        // escalating, so only a genuinely wedged holder is signalled.
        std::thread::sleep(RELAUNCH_TEARDOWN_GRACE);
        if !holder_still_owns_lock(lock_path, host, pid) {
            log::info!(
                "[cef-stale-reap] pid={pid} released the CEF lock within the relaunch grace; \
                 not reaping (normal post-update teardown)"
            );
            return;
        }
        log::warn!(
            "[cef-stale-reap] recent post-update marker + live CEF-lock holder pid={pid}; \
             still wedged after relaunch grace, sending SIGTERM"
        );
        if let Err(e) = kill_pid_term(pid_u32) {
            log::warn!("[cef-stale-reap] SIGTERM pid={pid} failed: {e}");
        }

        std::thread::sleep(TERM_GRACE);

        if holder_still_owns_lock(lock_path, host, pid) {
            log::warn!(
                "[cef-stale-reap] pid={pid} still holds the CEF lock after SIGTERM+grace; SIGKILL"
            );
            if let Err(e) = kill_pid_force(pid_u32) {
                log::warn!("[cef-stale-reap] SIGKILL pid={pid} failed: {e}");
            }
        } else {
            // Either it exited on SIGTERM, or the lock now points elsewhere.
            // Do NOT force-kill — the pid may have been reused. `cef_preflight`
            // removes the now-dead lock on its next poll.
            log::info!(
                "[cef-stale-reap] pid={pid} released the CEF lock after SIGTERM (or ownership changed); no SIGKILL"
            );
        }
    }

    /// Read+parse the live `SingletonLock` holder as `(host, pid)`. Returns
    /// `None` when the lock is absent, unparseable, or the pid is not alive
    /// (a dead-pid lock is stale and handled by `cef_preflight`).
    fn live_lock_holder(lock_path: &Path) -> Option<(String, i32)> {
        let target = std::fs::read_link(lock_path).ok()?;
        let (host, pid) = cef_preflight::parse_lock_target(&target.to_string_lossy())?;
        cef_preflight::is_pid_alive(pid).then_some((host, pid))
    }

    /// True iff the lock still resolves to `expected_pid` and that pid is alive.
    fn holder_still_owns_lock(lock_path: &Path, expected_host: &str, expected_pid: i32) -> bool {
        let Ok(target) = std::fs::read_link(lock_path) else {
            return false;
        };
        match cef_preflight::parse_lock_target(&target.to_string_lossy()) {
            // Revalidate the full (host, pid) owner, not pid alone: on a shared
            // cache path a reused numeric pid on another host must not be
            // mistaken for the holder we decided to reap.
            Some((host, pid)) => {
                host == expected_host && pid == expected_pid && cef_preflight::is_pid_alive(pid)
            }
            None => false,
        }
    }

    /// This machine's hostname, matching the `<hostname>-<pid>` that Chromium
    /// writes into `SingletonLock`. `None` if it cannot be resolved.
    fn local_hostname() -> Option<String> {
        nix::unistd::gethostname()
            .ok()?
            .into_string()
            .ok()
            .filter(|h| !h.is_empty())
    }

    /// Read the marker's recorded pid + freshness and delete it (one-shot).
    /// Returns the pid only when the marker existed, is within
    /// [`MARKER_MAX_AGE`], and parses — that pid must match the live lock holder
    /// before any reap, so a leaked marker cannot target an unrelated process.
    fn consume_marker(marker_path: &Path) -> Option<i32> {
        let result = match std::fs::metadata(marker_path) {
            Ok(meta) => {
                let age = meta
                    .modified()
                    .ok()
                    .and_then(|m| std::time::SystemTime::now().duration_since(m).ok());
                let fresh = marker_is_fresh(age, MARKER_MAX_AGE);
                let pid = std::fs::read_to_string(marker_path)
                    .ok()
                    .and_then(|body| parse_marker_pid(&body));
                log::info!(
                    "[cef-stale-reap] update-relaunch marker present at {} (fresh={fresh}, pid={pid:?})",
                    marker_path.display()
                );
                if fresh {
                    pid
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        // Delete whether fresh or stale so it is never reused.
        if let Err(e) = std::fs::remove_file(marker_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "[cef-stale-reap] failed to remove update-relaunch marker {}: {e}",
                    marker_path.display()
                );
            }
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::symlink;
        use std::path::PathBuf;

        // Serialises tests that mutate the process-global OPENHUMAN_CEF_CACHE_PATH.
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn fresh_tmp(tag: &str) -> PathBuf {
            let tmp = std::env::temp_dir().join(format!(
                "oh-cef-stale-reap-{}-{}-{}",
                tag,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).expect("create tmp dir");
            tmp
        }

        #[test]
        fn live_lock_holder_reads_self_pid() {
            let tmp = fresh_tmp("live-holder");
            let me = std::process::id() as i32;
            symlink(format!("myhost-{me}"), tmp.join("SingletonLock")).unwrap();

            let holder = live_lock_holder(&tmp.join("SingletonLock"));
            assert_eq!(holder, Some(("myhost".to_string(), me)));
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn live_lock_holder_none_for_dead_pid() {
            let tmp = fresh_tmp("dead-holder");
            // ~i32::MAX-1: far beyond any plausible live pid.
            symlink("deadhost-2147483646", tmp.join("SingletonLock")).unwrap();
            assert_eq!(live_lock_holder(&tmp.join("SingletonLock")), None);
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn holder_still_owns_lock_matches_same_pid_only() {
            let tmp = fresh_tmp("revalidate");
            let me = std::process::id() as i32;
            let lock = tmp.join("SingletonLock");
            symlink(format!("myhost-{me}"), &lock).unwrap();

            assert!(holder_still_owns_lock(&lock, "myhost", me));
            // A different (dead) pid must not re-validate.
            assert!(!holder_still_owns_lock(&lock, "myhost", 2147483646));
            // Same numeric pid on a different host must not re-validate either
            // (shared/NFS cache-path pid reuse).
            assert!(!holder_still_owns_lock(&lock, "otherhost", me));
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn consume_marker_reports_fresh_and_deletes() {
            let tmp = fresh_tmp("consume-fresh");
            let cache = tmp.join("cef");
            std::fs::create_dir_all(&cache).unwrap();
            let marker = marker_path_for(&cache);
            std::fs::write(&marker, "pid=1\n").unwrap();

            assert_eq!(
                consume_marker(&marker),
                Some(1),
                "just-written marker returns its recorded pid"
            );
            assert!(
                std::fs::metadata(&marker).is_err(),
                "marker must be deleted after consume"
            );
            // Second consume: absent → no pid, no error.
            assert_eq!(consume_marker(&marker), None);
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn write_then_consume_roundtrip_via_env() {
            let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prior = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH");

            let tmp = fresh_tmp("roundtrip");
            let cache = tmp.join("cef");
            std::fs::create_dir_all(&cache).unwrap();
            std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", &cache);

            super::super::write_update_relaunch_marker();
            let marker = marker_path_for(&cache);
            assert!(
                std::fs::metadata(&marker).is_ok(),
                "marker must exist after write"
            );
            assert_eq!(
                consume_marker(&marker),
                Some(std::process::id() as i32),
                "freshly written marker returns this process's pid"
            );

            match prior {
                Some(v) => std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", v),
                None => std::env::remove_var("OPENHUMAN_CEF_CACHE_PATH"),
            }
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn reap_orchestrator_no_lock_no_marker_is_safe_noop() {
            // With no SingletonLock and no marker, the orchestrator must take
            // the Skip path — no panic, no files created, nothing killed.
            let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prior = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH");

            let tmp = fresh_tmp("noop");
            let cache = tmp.join("cef");
            std::fs::create_dir_all(&cache).unwrap();
            std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", &cache);

            reap_stale_cef_lock_holder();

            assert!(
                std::fs::metadata(marker_path_for(&cache)).is_err(),
                "orchestrator must not create a marker"
            );

            match prior {
                Some(v) => std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", v),
                None => std::env::remove_var("OPENHUMAN_CEF_CACHE_PATH"),
            }
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use imp::reap_stale_cef_lock_holder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_when_no_holder() {
        assert_eq!(
            decide_reap(None, 100, Some("host"), Some(1)),
            ReapDecision::Skip("no live CEF SingletonLock holder")
        );
    }

    #[test]
    fn skip_when_holder_is_self() {
        assert_eq!(
            decide_reap(Some(("host".into(), 100)), 100, Some("host"), Some(100)),
            ReapDecision::Skip("CEF lock held by self")
        );
    }

    #[test]
    fn skip_when_holder_on_other_host() {
        assert_eq!(
            decide_reap(Some(("otherbox".into(), 200)), 100, Some("host"), Some(200)),
            ReapDecision::Skip("CEF lock holder is on a different host")
        );
    }

    #[test]
    fn skip_when_no_fresh_marker_even_if_live_holder() {
        // The core safety property: a live holder on our host is NOT reaped
        // without a recent post-update marker (it may be a healthy instance).
        assert_eq!(
            decide_reap(Some(("host".into(), 200)), 100, Some("host"), None),
            ReapDecision::Skip("no recent post-update marker; deferring to preflight wait")
        );
    }

    #[test]
    fn reap_only_with_marker_matching_host_and_other_pid() {
        assert_eq!(
            decide_reap(Some(("host".into(), 200)), 100, Some("host"), Some(200)),
            ReapDecision::Reap {
                host: "host".into(),
                pid: 200
            }
        );
    }

    #[test]
    fn skip_when_marker_pid_does_not_match_holder() {
        // A fresh marker recorded a *different* pid than the current lock holder
        // (e.g. a leaked marker): must not authorize killing this holder.
        assert_eq!(
            decide_reap(Some(("host".into(), 200)), 100, Some("host"), Some(999)),
            ReapDecision::Skip("post-update marker pid does not match the live CEF-lock holder")
        );
    }

    #[test]
    fn skip_when_local_host_unresolved() {
        // Fail closed: an unresolved local hostname means we cannot prove the
        // holder is same-host, so we must NOT reap even with a matching marker.
        assert_eq!(
            decide_reap(Some(("host".into(), 200)), 100, None, Some(200)),
            ReapDecision::Skip("local hostname unresolved; cannot prove same-host lock holder")
        );
    }

    #[test]
    fn marker_is_fresh_bounds_on_age() {
        assert!(marker_is_fresh(
            Some(Duration::from_secs(0)),
            MARKER_MAX_AGE
        ));
        assert!(marker_is_fresh(Some(MARKER_MAX_AGE), MARKER_MAX_AGE));
        assert!(!marker_is_fresh(
            Some(MARKER_MAX_AGE + Duration::from_secs(1)),
            MARKER_MAX_AGE
        ));
        // Unknown age is fail-safe (not fresh → no reap).
        assert!(!marker_is_fresh(None, MARKER_MAX_AGE));
    }

    #[test]
    fn marker_path_is_sibling_of_cache_dir() {
        assert_eq!(
            marker_path_for(Path::new("/x/com.openhuman.app/cef")),
            PathBuf::from("/x/com.openhuman.app/openhuman-update-relaunch.marker")
        );
    }
}
