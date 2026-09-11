
impl AuthProfilesStore {
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
    fn reclaim_self_owned_lock(&self) -> bool {
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
