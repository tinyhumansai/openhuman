//! Cross-process exclusive locking for the auth-profile store: acquisition,
//! stale-lock reclamation, and same-process leaked-lock self-healing.

use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::atomic::Ordering;

use super::{
    annotate_lock_create_failure, in_process_lock_for, is_pid_alive, AuthProfileLockGuard,
    AuthProfilesStore, LOCK_TIMEOUT_MS, LOCK_WAIT_MS, MALFORMED_LOCK_GRACE_MS, STALE_LOCK_AGE_MS,
};

impl AuthProfilesStore {
    pub(super) fn acquire_lock(&self) -> Result<AuthProfileLockGuard> {
        // Test-only: simulate a full / read-only filesystem that can't create
        // the lock file, to drive the read-only fallback in `load`.
        #[cfg(test)]
        if self.force_lock_unwritable.swap(false, Ordering::SeqCst) {
            let io = std::io::Error::from(std::io::ErrorKind::StorageFull);
            return Err(annotate_lock_create_failure(
                anyhow::Error::new(io).context("open lock file"),
            ));
        }

        if let Some(parent) = self.lock_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| "Failed to create auth profile lock directory".to_string())?;
        }

        // Drive timeout + stale-recheck off wall-clock elapsed time, not the
        // sum of explicit `thread::sleep(LOCK_WAIT_MS)` calls. The earlier
        // counter-based approach excluded time spent inside
        // `retry_with_backoff` (which can sleep up to ~30s on its own
        // schedule before returning AlreadyExists) and the lock-file I/O
        // syscalls. Under Windows AV contention that drift could push
        // both `LOCK_TIMEOUT_MS` and `next_stale_recheck_ms` significantly
        // later than intended.
        let started_at = Instant::now();

        // Serialize same-process acquirers on an in-memory lock keyed by this
        // path before we ever touch the on-disk lock file. Two things depend on
        // holding it: (1) concurrent `app_state_snapshot` calls in this process
        // queue here instead of racing `create_new`/`Drop` on the file, and
        // (2) it lets us treat an on-disk lock recording our own pid as a leaked
        // `Drop` unlink and reclaim it immediately — no other thread in this
        // process can hold a live guard while we own this in-memory lock (see
        // `reclaim_self_owned_lock`). Held for the lifetime of the returned
        // guard.
        //
        // Use `try_lock` against the *same* `LOCK_TIMEOUT_MS` budget as the
        // on-disk wait rather than a blocking `lock()`: a wedged in-process
        // holder must not be able to strand an RPC/blocking worker past the
        // timeout the caller (e.g. `app_state_snapshot`) expects. Poison is
        // recoverable — the `()` payload carries no invariant.
        let in_process_lock = in_process_lock_for(&self.lock_path);
        let in_process_guard = in_process_lock.acquire(Duration::from_millis(LOCK_TIMEOUT_MS))?;

        let mut cleared_stale = false;
        // Periodically re-probe for stale locks during the busy-wait. A
        // lock that started fresh (live pid, recent mtime) can age past
        // STALE_LOCK_AGE_MS while we wait, and we want to recover from
        // that without bailing at the LOCK_TIMEOUT_MS boundary.
        let mut next_stale_recheck_ms: u64 = 1_000;
        loop {
            let open_result =
                crate::util::retry_with_backoff("create auth profile lock", 6, 100, || {
                    OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&self.lock_path)
                        .context("open lock file")
                });

            match open_result {
                Ok(mut file) => {
                    // Issue #1612 — writing the pid line is what later lets
                    // a future acquirer recognise a crashed owner; if the
                    // write fails we must NOT report the lock as held with
                    // a malformed/empty file behind us, or stale recovery
                    // would silently degrade to the full 10s timeout for
                    // every subsequent acquire.
                    if let Err(e) = writeln!(file, "pid={}", std::process::id()) {
                        let _ = fs::remove_file(&self.lock_path);
                        return Err(e).with_context(|| {
                            "Failed to write auth profile lock owner".to_string()
                        });
                    }
                    return Ok(AuthProfileLockGuard {
                        lock_path: self.lock_path.clone(),
                        _in_process: in_process_guard,
                    });
                }
                Err(e) => {
                    let is_already_exists = e
                        .chain()
                        .find_map(|e| e.downcast_ref::<std::io::Error>())
                        .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::AlreadyExists);

                    if is_already_exists {
                        // A lock file recording our own pid can only be a
                        // leaked `Drop` unlink: we hold the in-process lock, so
                        // no live same-process guard exists. Reclaim it
                        // immediately rather than spinning the 30s age floor
                        // (`STALE_LOCK_AGE_MS`) that sits *behind* the ~10s RPC
                        // timeout — that gap is what produced the sustained
                        // "Timed out waiting for auth profile lock" retry storm
                        // (Sentry TAURI-RUST-B1 / #2318). Cheap enough (one tiny
                        // read) to re-probe every spin, which also self-heals a
                        // lock our own `Drop` leaks mid-wait.
                        if self.reclaim_self_owned_lock() {
                            continue;
                        }
                        // Issue #1612 — a previous openhuman crash can leave a
                        // stale auth-profiles.lock behind (a *different*, now-dead
                        // pid, or an aged leak), after which every RPC path that
                        // touches the auth profile store fails for the
                        // `LOCK_TIMEOUT_MS` window and the user gets stuck in a
                        // retry storm. Before falling back to the busy-wait, try
                        // once to peek at the writer's recorded PID and remove
                        // the lock if that process is no longer alive. Flag is
                        // flipped on the first probe (not only on success) so a
                        // live-pid / malformed / unreadable lock doesn't trigger
                        // a fresh sysinfo probe + log line on every busy-wait
                        // iteration.
                        if !cleared_stale {
                            cleared_stale = true;
                            if self.clear_lock_if_stale() {
                                continue;
                            }
                        } else {
                            let elapsed_ms = started_at.elapsed().as_millis() as u64;
                            if elapsed_ms >= next_stale_recheck_ms {
                                // The age-based reclaim check is cheap (one
                                // `fs::metadata` call in the common case) and
                                // safely no-ops on fresh, legitimate locks.
                                // Re-probing periodically lets us recover from
                                // a leaked-mid-wait lock without bailing at
                                // the 10s timeout.
                                next_stale_recheck_ms = next_stale_recheck_ms.saturating_add(1_000);
                                if self.clear_lock_if_stale() {
                                    continue;
                                }
                            }
                        }
                        if started_at.elapsed().as_millis() as u64 >= LOCK_TIMEOUT_MS {
                            anyhow::bail!("Timed out waiting for auth profile lock");
                        }
                        thread::sleep(Duration::from_millis(LOCK_WAIT_MS));
                    } else {
                        // Sentry OPENHUMAN-TAURI-H8 collapses every
                        // non-AlreadyExists, non-transient `create_new`
                        // failure into a single fingerprint with no
                        // breadcrumb of which OS code actually fired.
                        // `annotate_lock_create_failure` embeds the
                        // underlying `io::ErrorKind` + `raw_os_error()` so
                        // future events split by root cause and we can
                        // widen `is_transient_fs_error` (or fix the
                        // underlying condition) for whichever code is hot.
                        return Err(annotate_lock_create_failure(e));
                    }
                }
            }
        }
    }

    /// Returns `true` if an existing lock file was detected as stale and
    /// successfully removed. Two cases reclaim:
    ///
    /// 1. The recorded `pid=` line points at a process that is no longer
    ///    running — classic crashed-owner recovery (Issue #1612).
    /// 2. The lock file's mtime is older than [`STALE_LOCK_AGE_MS`]. This
    ///    catches a *different* still-alive process that leaked its lock (its
    ///    `AuthProfileLockGuard::drop` could not unlink the file — e.g. Windows
    ///    AV / indexer briefly held a handle — and orphaned it with that live
    ///    pid inside). No legitimate auth-profile op holds the lock long enough
    ///    to be affected, so a too-old lock is unambiguously a leak. A lock
    ///    leaked by *this* process is handled far sooner — immediately, without
    ///    the age floor — by [`reclaim_self_owned_lock`](Self::reclaim_self_owned_lock),
    ///    which is sound because acquirers serialize on the in-process lock.
    ///
    /// 3. The lock file has no parseable `pid=` line and is older than
    ///    [`MALFORMED_LOCK_GRACE_MS`]. A healthy holder writes its pid within
    ///    microseconds of `create_new`, so a pidless lock past that short
    ///    grace is an abandoned in-flight writer (crashed/killed between
    ///    `create_new` and the `pid=` write) — reclaim it rather than make
    ///    every reader spin the full [`STALE_LOCK_AGE_MS`]/`LOCK_TIMEOUT_MS`
    ///    window (the ~30s "stuck on Initializing OpenHuman" after a
    ///    kill+reopen). The grace is short but non-zero so we never reclaim a
    ///    live writer that is mid-`create_new`/`pid=`.
    pub(super) fn clear_lock_if_stale(&self) -> bool {
        let metadata = match fs::metadata(&self.lock_path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(e) => {
                tracing::warn!(
                    target: "auth-profiles",
                    "[credentials] failed to stat lock file at {} for stale check: {e}",
                    self.lock_path.display()
                );
                return false;
            }
        };

        let age = metadata
            .modified()
            .ok()
            .and_then(|mtime| std::time::SystemTime::now().duration_since(mtime).ok());
        let too_old = age.is_some_and(|a| a >= Duration::from_millis(STALE_LOCK_AGE_MS));
        // A pidless lock needs only a short grace: no healthy holder leaves the
        // file without a `pid=` line for more than the microsecond gap between
        // `create_new` and the write, so anything older is abandoned. If mtime
        // is unreadable (clock skew, platform limitation) default to stale —
        // no legitimate in-flight writer would be undetectable for that long.
        let malformed_too_old =
            age.is_none_or(|a| a >= Duration::from_millis(MALFORMED_LOCK_GRACE_MS));

        let content = match fs::read_to_string(&self.lock_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(e) => {
                tracing::warn!(
                    target: "auth-profiles",
                    "[credentials] failed to read lock file at {} for stale check: {e}",
                    self.lock_path.display()
                );
                return false;
            }
        };

        let pid = content
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid=")?.trim().parse::<u32>().ok());

        let reclaim_reason: Option<String> = match pid {
            Some(pid) if !is_pid_alive(pid) => Some(format!("pid {pid} not alive")),
            Some(pid) if too_old => Some(format!(
                "lock file older than {STALE_LOCK_AGE_MS}ms (recorded pid {pid}, presumed leaked)"
            )),
            None if malformed_too_old => Some(format!(
                "no parseable pid and older than {MALFORMED_LOCK_GRACE_MS}ms \
                 (abandoned in-flight lock, reclaiming)"
            )),
            Some(_) => return false,
            None => {
                tracing::warn!(
                    target: "auth-profiles",
                    "[credentials] lock at {} has no parseable pid line and is younger than \
                     {MALFORMED_LOCK_GRACE_MS}ms; leaving in place briefly",
                    self.lock_path.display()
                );
                return false;
            }
        };

        let Some(reason) = reclaim_reason else {
            return false;
        };

        match fs::remove_file(&self.lock_path) {
            Ok(()) => {
                tracing::info!(
                    target: "auth-profiles",
                    "[credentials] removed stale auth profile lock at {} ({reason})",
                    self.lock_path.display()
                );
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                tracing::warn!(
                    target: "auth-profiles",
                    "[credentials] failed to remove stale lock at {} ({reason}): {e}",
                    self.lock_path.display()
                );
                false
            }
        }
    }

    /// Remove the on-disk lock file **iff** it records this process's own pid.
    /// Returns `true` when a self-owned lock was found and removed.
    ///
    /// MUST only be called while holding this path's in-process lock (acquired
    /// at the top of [`acquire_lock`](Self::acquire_lock) via
    /// [`in_process_lock_for`]). That invariant is what makes the removal safe:
    /// same-process acquirers serialize on the in-process lock, so no other
    /// thread in this process can be holding a live `AuthProfileLockGuard` while
    /// we run. A lock file carrying our own pid is therefore necessarily a leak
    /// — a previous guard's `Drop` could not unlink it (on Windows, AV / the
    /// Search indexer briefly hold a handle on the just-written file) and
    /// orphaned it with our still-alive pid inside.
    ///
    /// This is the fast self-heal for Sentry TAURI-RUST-B1 (#2318): without it
    /// the only recovery for a leaked-but-live-pid lock is the age branch in
    /// [`clear_lock_if_stale`](Self::clear_lock_if_stale), whose
    /// [`STALE_LOCK_AGE_MS`] floor (30s) is far longer than the ~10s RPC
    /// timeout, so every `app_state_snapshot` timed out and retry-stormed for
    /// the whole 30s window before age-reclaim kicked in.
    pub(crate) fn reclaim_self_owned_lock(&self) -> bool {
        let me = std::process::id();
        tracing::trace!(
            target: "auth-profiles",
            "[credentials] probing for self-owned leaked lock at {} (our pid {me})",
            self.lock_path.display()
        );
        let content = match fs::read_to_string(&self.lock_path) {
            Ok(s) => s,
            // NotFound: already gone (raced with another reclaim / the guard's
            // own Drop). Any other read error: fall back to the age-based and
            // busy-wait paths rather than guess at ownership.
            Err(e) => {
                tracing::debug!(
                    target: "auth-profiles",
                    "[credentials] self-owned probe could not read lock at {} ({e}); \
                     deferring to stale/busy-wait recovery",
                    self.lock_path.display()
                );
                return false;
            }
        };

        let pid = content
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid=")?.trim().parse::<u32>().ok());
        if pid != Some(me) {
            tracing::trace!(
                target: "auth-profiles",
                "[credentials] lock at {} is not self-owned (recorded {pid:?}, our pid {me}); \
                 deferring to stale/busy-wait recovery",
                self.lock_path.display()
            );
            return false;
        }

        match fs::remove_file(&self.lock_path) {
            Ok(()) => {
                tracing::info!(
                    target: "auth-profiles",
                    "[credentials] reclaimed leaked self-owned auth profile lock at {} \
                     (recorded our own live pid {me}; a prior guard's Drop unlink was blocked, \
                     most likely by Windows AV/indexer)",
                    self.lock_path.display()
                );
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                tracing::warn!(
                    target: "auth-profiles",
                    "[credentials] failed to reclaim self-owned auth profile lock at {}: {e}",
                    self.lock_path.display()
                );
                false
            }
        }
    }
}
