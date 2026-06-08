//! CEF cache-lock preflight check (macOS and Linux).
//!
//! When another OpenHuman instance is already running, it holds an exclusive
//! lock on the CEF user-data-dir. On macOS this is
//! `~/Library/Caches/com.openhuman.app/cef`; on Linux it is the path in
//! `OPENHUMAN_CEF_CACHE_PATH` (set by `cef_profile::prepare_process_cache_path`
//! before this module runs), falling back to `$XDG_CACHE_HOME/<id>/cef` or
//! `$HOME/.cache/<id>/cef` when the env var is absent.
//!
//! The vendored `tauri-runtime-cef` crate calls `cef::initialize()` and
//! asserts the result equals `1`; on lock collision it returns `0` and the
//! assertion panics with a Rust backtrace and no actionable message
//! (Sentry OPENHUMAN-TAURI-K1 on Linux, issue #864 on macOS).
//!
//! This module runs *before* the Tauri builder constructs the runtime.
//! It detects the lock-holder PID via Chromium's `SingletonLock` symlink and
//! either:
//!   - returns [`CefLockError::Held`] when a live process owns the lock, or
//!   - removes a stale lock (PID no longer alive) and returns Ok.
//!
//! Stale-lock cleanup mirrors Chromium's own startup behavior so dev startup
//! is not blocked by crashed processes.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;

/// Maximum time to wait for a live lock-holder to release the CEF cache before
/// giving up. Mirrors the Windows `cef_singleton_wait` budget so the
/// relaunch-race behaviour is symmetric across platforms — kept short so a
/// genuinely stuck holder can't hang startup; the common race clears well
/// under a second.
const WAIT_BUDGET: Duration = Duration::from_secs(5);
/// First poll backoff; doubles each attempt up to [`BACKOFF_CAP`].
const BACKOFF_BASE: Duration = Duration::from_millis(100);
/// Upper bound on a single backoff sleep so we keep polling responsively.
const BACKOFF_CAP: Duration = Duration::from_millis(500);

/// Bundle identifier from `tauri.conf.json`. Must match `bundle.identifier` —
/// the vendored `tauri-runtime-cef` derives the cache directory as
/// `dirs::cache_dir() / <identifier> / cef`. If `tauri.conf.json` ever changes
/// the bundle identifier, update this constant too.
pub const APP_IDENTIFIER: &str = "com.openhuman.app";

/// Errors returned by the preflight check.
#[derive(Debug)]
pub enum CefLockError {
    /// Another live process holds the CEF cache lock.
    Held {
        pid: i32,
        host: String,
        cache_path: PathBuf,
    },
    /// `$HOME` not set — cannot resolve default cache path. Treated as
    /// non-fatal at the call site (preflight is best-effort).
    NoHomeDir,
}

impl fmt::Display for CefLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held {
                pid,
                host,
                cache_path,
            } => {
                // The force-quit hint is platform-specific: macOS runs from an
                // `.app` bundle, Linux from a plain binary. `cef_preflight` is
                // compiled for both (`cfg(any(macos, linux))`), so a hardcoded
                // macOS `pkill` pattern would mislead Linux users.
                let workaround = if cfg!(target_os = "macos") {
                    "pkill -f \"OpenHuman.app/Contents\"\n  pkill -f \"openhuman-core\""
                } else {
                    "pkill -f openhuman\n  pkill -f openhuman-core"
                };
                write!(
                    f,
                    "CEF cache at {} is held by another OpenHuman instance \
                     (host {}, pid {}).\n\
                     Quit the running instance and try again.\n\
                     Workaround:\n  {}",
                    cache_path.display(),
                    host,
                    pid,
                    workaround,
                )
            }
            Self::NoHomeDir => write!(
                f,
                "$HOME not set — cannot resolve CEF cache path for preflight"
            ),
        }
    }
}

impl std::error::Error for CefLockError {}

/// Resolves the platform default CEF cache directory and runs the preflight.
///
/// Checks `OPENHUMAN_CEF_CACHE_PATH` first (always set by
/// `cef_profile::prepare_process_cache_path` before this runs). Falls back
/// to the platform-specific default: `~/Library/Caches/<id>/cef` on macOS,
/// `$XDG_CACHE_HOME/<id>/cef` or `$HOME/.cache/<id>/cef` on Linux.
pub fn check_default_cache() -> Result<(), CefLockError> {
    if let Some(configured) = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH") {
        let configured = PathBuf::from(configured);
        log::debug!(
            "[cef-preflight] using configured cache_path={}",
            configured.display()
        );
        return check_cef_cache_lock(&configured);
    }

    let home = std::env::var_os("HOME").ok_or(CefLockError::NoHomeDir)?;
    let home = PathBuf::from(home);

    #[cfg(target_os = "macos")]
    let cache_path = home.join("Library/Caches").join(APP_IDENTIFIER).join("cef");

    // On Linux: $XDG_CACHE_HOME/<id>/cef or $HOME/.cache/<id>/cef.
    // This matches the fallback path in tauri-runtime-cef's CefRuntime::init
    // (via `dirs::cache_dir()`).
    #[cfg(target_os = "linux")]
    let cache_path = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".cache"))
        .join(APP_IDENTIFIER)
        .join("cef");

    log::debug!("[cef-preflight] cache_path={}", cache_path.display());
    check_cef_cache_lock(&cache_path)
}

/// Inspects `<cache_path>/SingletonLock` (Chromium symlink). If present and
/// the target PID is still alive, returns [`CefLockError::Held`]. If the lock
/// is stale (PID dead), removes it and returns Ok — matches Chromium's own
/// startup recovery behavior.
pub fn check_cef_cache_lock(cache_path: &Path) -> Result<(), CefLockError> {
    let lock_path = cache_path.join("SingletonLock");

    // `symlink_metadata` does not follow symlinks — we want to know whether
    // the symlink itself exists. CEF/Chromium lays this down as a symlink
    // whose target string encodes the lock-holder.
    let meta = match fs::symlink_metadata(&lock_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "[cef-preflight] no SingletonLock at {}",
                lock_path.display()
            );
            return Ok(());
        }
        Err(e) => {
            log::warn!(
                "[cef-preflight] cannot stat {}: {} — assuming no lock",
                lock_path.display(),
                e
            );
            return Ok(());
        }
    };

    if !meta.file_type().is_symlink() {
        log::warn!(
            "[cef-preflight] {} exists but is not a symlink — skipping check",
            lock_path.display()
        );
        return Ok(());
    }

    let target = match fs::read_link(&lock_path) {
        Ok(t) => t,
        Err(e) => {
            log::warn!(
                "[cef-preflight] cannot read symlink {}: {} — skipping check",
                lock_path.display(),
                e
            );
            return Ok(());
        }
    };

    let target_str = target.to_string_lossy();
    let Some((host, pid)) = parse_lock_target(&target_str) else {
        log::warn!(
            "[cef-preflight] unrecognized lock target format: {:?}",
            target_str
        );
        return Ok(());
    };

    if is_pid_alive(pid) {
        // `debug!`, not `error!`: a live lock-holder is the *expected,
        // self-healing* precondition the caller polls on, not a fault. This is a
        // pure predicate — severity belongs to the loop, not here. #3337 wrapped
        // this check in the `wait_for_cache_release` poll loop, so an `error!`
        // here fired once *per poll* (~3 on a wait that clears, ~11 on one that
        // times out), flooding Sentry (TAURI-RUST-B7S: 29k events from a single
        // release). The loop already logs `warn!` per poll and a single `error!`
        // on give-up, matching the Windows `cef_singleton_wait` sibling. Do not
        // re-promote this to `error!`.
        log::debug!(
            "[cef-preflight] CEF cache held by host={} pid={} at {}",
            host,
            pid,
            cache_path.display()
        );
        return Err(CefLockError::Held {
            pid,
            host,
            cache_path: cache_path.to_path_buf(),
        });
    }

    log::warn!(
        "[cef-preflight] removing stale lock at {} (pid {} not alive)",
        lock_path.display(),
        pid
    );
    if let Err(e) = fs::remove_file(&lock_path) {
        log::warn!(
            "[cef-preflight] failed to remove stale lock {}: {}",
            lock_path.display(),
            e
        );
    }
    Ok(())
}

/// Parses Chromium's `SingletonLock` symlink target — `<hostname>-<pid>`.
/// Hostnames may contain dashes; the rightmost dash is the separator.
pub fn parse_lock_target(target: &str) -> Option<(String, i32)> {
    let (host, pid_str) = target.rsplit_once('-')?;
    let pid: i32 = pid_str.parse().ok()?;
    if host.is_empty() || pid <= 0 {
        return None;
    }
    Some((host.to_string(), pid))
}

/// Returns true iff a PID is still a live process visible to us. Sends signal
/// 0 (POSIX existence check) — does not actually deliver a signal.
pub fn is_pid_alive(pid: i32) -> bool {
    matches!(kill(Pid::from_raw(pid), None), Ok(()))
}

/// Exponential backoff for poll `attempt` (0-based), capped at [`BACKOFF_CAP`].
/// Pure + saturating so it never overflows on a high attempt count. Mirrors
/// the Windows `cef_singleton_wait` backoff.
fn backoff_delay(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
    BACKOFF_BASE
        .checked_mul(factor)
        .unwrap_or(BACKOFF_CAP)
        .min(BACKOFF_CAP)
}

/// Bounded wait for the CEF cache lock to clear (macOS + Linux).
///
/// The one-shot [`check_default_cache`] used to `exit(1)` the moment it saw a
/// live lock-holder. But the dominant TAURI-RUST-F cause is a *sequential
/// relaunch race*: the prior instance is mid-teardown and releases the lock
/// within a second (auto-update relaunch, fast quit+reopen, a restart flow).
/// Polling for a short budget lets that resolve seamlessly instead of killing
/// the relaunch — the macOS/Linux analogue of the Windows pre-CEF wait.
///
/// Outcomes:
///   * lock clears (or is stale and removed) → return; caller proceeds to
///     `cef::initialize()`.
///   * cache path unresolved ([`CefLockError::NoHomeDir`]) → nothing to wait
///     on; proceed best-effort (matches the prior behaviour).
///   * lock still held by a live process after [`WAIT_BUDGET`] → a genuine
///     "another instance is running" situation: print the actionable message
///     and exit cleanly with code **0** (not a crash; `exit(1)` previously
///     made relaunch wrappers treat it as a failure).
///
/// This is defense-in-depth, never the only guard: any failure that still
/// reaches `cef::initialize()` (lock freed then re-acquired between the check
/// and init, or a GPU/sandbox/permission failure) is caught by the runtime's
/// `cef_init_guard`, which shows a dialog and exits cleanly. Together they make
/// the `assertion left == right` panic impossible regardless of cause.
pub fn wait_for_cache_release() {
    let start = Instant::now();
    let mut attempt: u32 = 0;

    loop {
        match check_default_cache() {
            Ok(()) => {
                if attempt > 0 {
                    log::info!(
                        "[cef-preflight] CEF cache lock released after {} poll(s) ({} ms); proceeding to CEF init",
                        attempt,
                        start.elapsed().as_millis()
                    );
                }
                return;
            }
            Err(CefLockError::NoHomeDir) => {
                log::warn!("[cef-preflight] {}", CefLockError::NoHomeDir);
                return;
            }
            Err(held) => {
                let elapsed = start.elapsed();
                if elapsed >= WAIT_BUDGET {
                    log::error!(
                        "[cef-preflight] CEF cache still held after {} ms budget; exiting cleanly instead of initializing into a locked cache (TAURI-RUST-F)",
                        WAIT_BUDGET.as_millis()
                    );
                    eprintln!("\n[openhuman] {held}\n");
                    std::process::exit(0);
                }
                // Clamp the backoff to the remaining budget so the final sleep
                // can't overshoot WAIT_BUDGET (the documented total-wait
                // contract) by up to BACKOFF_CAP.
                let remaining = WAIT_BUDGET.checked_sub(elapsed).unwrap_or_default();
                let delay = backoff_delay(attempt).min(remaining);
                log::warn!(
                    "[cef-preflight] CEF cache held by another instance; waiting {} ms before re-check (elapsed {} ms, TAURI-RUST-F)",
                    delay.as_millis(),
                    elapsed.as_millis()
                );
                std::thread::sleep(delay);
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    // Shared lock for all tests that mutate process-global env vars.
    // Each test previously had its own local `static ENV_LOCK`, allowing
    // concurrent test threads to race on OPENHUMAN_CEF_CACHE_PATH /
    // XDG_CACHE_HOME. A single module-level lock serialises them.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn parse_target_simple() {
        assert_eq!(
            parse_lock_target("myhost-12345"),
            Some(("myhost".into(), 12345))
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

    #[test]
    fn parse_target_with_dashes_in_host() {
        assert_eq!(
            parse_lock_target("my-fancy-host-99"),
            Some(("my-fancy-host".into(), 99))
        );
    }

    #[test]
    fn parse_target_pid_not_int() {
        assert_eq!(parse_lock_target("just-a-name"), None);
    }

    #[test]
    fn parse_target_empty_pid() {
        assert_eq!(parse_lock_target("host-"), None);
    }

    #[test]
    fn parse_target_empty_host() {
        assert_eq!(parse_lock_target("-12345"), None);
    }

    fn fresh_tmp(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "oh-cef-preflight-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create tmp dir");
        tmp
    }

    #[test]
    fn no_lock_returns_ok() {
        let tmp = fresh_tmp("nolock");
        assert!(check_cef_cache_lock(&tmp).is_ok());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lock_held_by_live_pid_returns_err() {
        let tmp = fresh_tmp("live");
        let me = std::process::id() as i32;
        symlink(format!("livehost-{me}"), tmp.join("SingletonLock")).unwrap();

        match check_cef_cache_lock(&tmp) {
            Err(CefLockError::Held { pid, host, .. }) => {
                assert_eq!(pid, me);
                assert_eq!(host, "livehost");
            }
            other => panic!("expected Held, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lock_stale_dead_pid_returns_ok_and_removes() {
        let tmp = fresh_tmp("stale");
        // PID 2147483646 (~i32::MAX-1) is far beyond any plausible live PID.
        symlink("deadhost-2147483646", tmp.join("SingletonLock")).unwrap();

        let lock = tmp.join("SingletonLock");
        assert!(
            fs::symlink_metadata(&lock).is_ok(),
            "lock should exist before"
        );

        let res = check_cef_cache_lock(&tmp);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
        assert!(
            fs::symlink_metadata(&lock).is_err(),
            "stale lock should have been removed"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lock_with_garbage_target_skips() {
        let tmp = fresh_tmp("garbage");
        symlink("not-a-valid-format", tmp.join("SingletonLock")).unwrap();

        // "not-a-valid-format" rsplit_once('-') -> ("not-a-valid", "format")
        // "format".parse::<i32>() fails -> parse_lock_target returns None ->
        // skipped, returns Ok and leaves the lock alone.
        let res = check_cef_cache_lock(&tmp);
        assert!(
            res.is_ok(),
            "expected Ok on unparseable target, got {res:?}"
        );
        assert!(
            fs::symlink_metadata(tmp.join("SingletonLock")).is_ok(),
            "unparseable lock must NOT be removed"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// `check_default_cache` must use `OPENHUMAN_CEF_CACHE_PATH` when set —
    /// on both macOS and Linux the profile module always sets this before the
    /// preflight runs, so the platform-specific fallback paths are irrelevant
    /// in production, but the configured-path branch must work on all platforms.
    #[test]
    fn check_default_cache_uses_configured_env_path() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prior = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH");
        let tmp = fresh_tmp("default-cache-env");

        std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", &tmp);
        let result = check_default_cache();

        match prior {
            Some(v) => std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", v),
            None => std::env::remove_var("OPENHUMAN_CEF_CACHE_PATH"),
        }

        assert!(result.is_ok(), "expected Ok with no lock, got {result:?}");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// `check_default_cache` with env-path pointing to a dir holding a live lock
    /// must return `CefLockError::Held`.
    #[test]
    fn check_default_cache_env_path_held_returns_err() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prior = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH");
        let tmp = fresh_tmp("default-cache-held");
        let me = std::process::id() as i32;
        symlink(format!("testhost-{me}"), tmp.join("SingletonLock")).unwrap();

        std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", &tmp);
        let result = check_default_cache();

        match prior {
            Some(v) => std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", v),
            None => std::env::remove_var("OPENHUMAN_CEF_CACHE_PATH"),
        }

        match result {
            Err(CefLockError::Held { pid, .. }) => assert_eq!(pid, me),
            other => panic!("expected Held, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    /// On Linux, `check_default_cache` without `OPENHUMAN_CEF_CACHE_PATH` set
    /// must fall back to `$XDG_CACHE_HOME/<id>/cef` and return Ok when no lock
    /// is present.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_default_cache_linux_xdg_fallback_no_lock() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let prior_cache = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH");
        let prior_xdg = std::env::var_os("XDG_CACHE_HOME");
        std::env::remove_var("OPENHUMAN_CEF_CACHE_PATH");

        // Redirect XDG_CACHE_HOME to a temp dir we control.
        let tmp = fresh_tmp("linux-xdg-fallback");
        std::env::set_var("XDG_CACHE_HOME", &tmp);

        let result = check_default_cache();

        std::env::remove_var("XDG_CACHE_HOME");
        match prior_cache {
            Some(v) => std::env::set_var("OPENHUMAN_CEF_CACHE_PATH", v),
            None => {}
        }
        match prior_xdg {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => {}
        }

        // No SingletonLock under tmp/<id>/cef — should be Ok.
        assert!(result.is_ok(), "expected Ok with no lock, got {result:?}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
