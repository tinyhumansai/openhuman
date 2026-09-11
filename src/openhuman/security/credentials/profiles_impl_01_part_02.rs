
impl AuthProfilesStore {
    fn save_locked(&self, data: &AuthProfilesData) -> Result<()> {
        let mut persisted = PersistedAuthProfiles {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: data.updated_at.to_rfc3339(),
            active_profiles: data.active_profiles.clone(),
            profiles: BTreeMap::new(),
        };

        for (id, profile) in &data.profiles {
            // When the OS keychain is available, store all secret fields there and
            // leave them absent from the JSON file.  This is the preferred path on
            // macOS / Windows / Linux-with-Secret-Service.
            //
            // When the keychain is unavailable (Linux headless / CI), fall back to
            // the existing ChaCha20-Poly1305 encrypted JSON fields.
            let (access_token, refresh_token, id_token, token, expires_at, token_type, scope) =
                if self.use_keychain {
                    // Store secrets in the OS keychain — JSON gets no secret fields.
                    if let Err(e) = self.keychain_store_secrets(profile) {
                        // Non-fatal: fall back to encrypted JSON so data is not lost.
                        log::warn!(
                            "[auth] save: keychain store failed for profile_id={id}: {e}; \
                             falling back to encrypted JSON"
                        );
                        self.encrypt_for_json(profile)?
                    } else {
                        log::debug!("[auth] save: secrets stored in keychain profile_id={id}");
                        let (expires_at, token_type, scope) = match &profile.token_set {
                            Some(ts) => (
                                ts.expires_at.as_ref().map(DateTime::to_rfc3339),
                                ts.token_type.clone(),
                                ts.scope.clone(),
                            ),
                            None => (None, None, None),
                        };
                        // Secret fields deliberately omitted from JSON.
                        (None, None, None, None, expires_at, token_type, scope)
                    }
                } else {
                    // Headless / no keychain — encrypt and store in JSON.
                    self.encrypt_for_json(profile)?
                };

            persisted.profiles.insert(
                id.clone(),
                PersistedAuthProfile {
                    provider: profile.provider.clone(),
                    profile_name: profile.profile_name.clone(),
                    kind: profile_kind_to_string(profile.kind).to_string(),
                    account_id: profile.account_id.clone(),
                    workspace_id: profile.workspace_id.clone(),
                    access_token,
                    refresh_token,
                    id_token,
                    token,
                    expires_at,
                    token_type,
                    scope,
                    metadata: profile.metadata.clone(),
                    created_at: profile.created_at.to_rfc3339(),
                    updated_at: profile.updated_at.to_rfc3339(),
                },
            );
        }

        self.write_persisted_locked(&persisted)
    }

    /// Encrypt a profile's secret fields for JSON storage (keychain-unavailable path).
    fn encrypt_for_json(&self, profile: &AuthProfile) -> Result<EncryptedProfileFields> {
        let (access_token, refresh_token, id_token, expires_at, token_type, scope) =
            match (&profile.kind, &profile.token_set) {
                (AuthProfileKind::OAuth, Some(token_set)) => (
                    self.encrypt_optional(Some(&token_set.access_token))?,
                    self.encrypt_optional(token_set.refresh_token.as_deref())?,
                    self.encrypt_optional(token_set.id_token.as_deref())?,
                    token_set.expires_at.as_ref().map(DateTime::to_rfc3339),
                    token_set.token_type.clone(),
                    token_set.scope.clone(),
                ),
                _ => (None, None, None, None, None, None),
            };
        let token = self.encrypt_optional(profile.token.as_deref())?;
        Ok((
            access_token,
            refresh_token,
            id_token,
            token,
            expires_at,
            token_type,
            scope,
        ))
    }

    fn read_persisted_locked(&self) -> Result<PersistedAuthProfiles> {
        if !self.path.exists() {
            return Ok(PersistedAuthProfiles::default());
        }

        let bytes = fs::read(&self.path).with_context(|| {
            format!(
                "Failed to read auth profile store at {}",
                self.path.display()
            )
        })?;

        if bytes.is_empty() {
            return Ok(PersistedAuthProfiles::default());
        }

        let mut persisted: PersistedAuthProfiles = match serde_json::from_slice(&bytes) {
            Ok(p) => p,
            Err(err) => {
                let quarantined = quarantine_corrupt_store(&self.path)?;
                let quarantined_file = quarantined
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("auth-profiles.corrupt");
                tracing::warn!(
                    path_file = PROFILES_FILENAME,
                    quarantined_file = quarantined_file,
                    error = %err,
                    "[credentials] auth profile store unparseable; quarantined and reset to empty"
                );
                return Ok(PersistedAuthProfiles::default());
            }
        };

        if persisted.schema_version == 0 {
            persisted.schema_version = CURRENT_SCHEMA_VERSION;
        }

        if persisted.schema_version > CURRENT_SCHEMA_VERSION {
            anyhow::bail!(
                "Unsupported auth profile schema version {} (max supported: {})",
                persisted.schema_version,
                CURRENT_SCHEMA_VERSION
            );
        }

        Ok(persisted)
    }

    fn write_persisted_locked(&self, persisted: &PersistedAuthProfiles) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create auth profile directory at {}",
                    parent.display()
                )
            })?;
        }

        let json =
            serde_json::to_vec_pretty(persisted).context("Failed to serialize auth profiles")?;
        let tmp_name = format!(
            "{}.tmp.{}.{}",
            PROFILES_FILENAME,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let tmp_path = self.path.with_file_name(tmp_name);

        // Windows AV / Search-Indexer / Defender may briefly hold a handle on
        // the destination, returning transient `ERROR_SHARING_VIOLATION (32)`,
        // `ERROR_ACCESS_DENIED (5)`, or `ERROR_DELETE_PENDING (303)` —
        // recognised as retryable by `is_transient_fs_error`. Mirror the
        // lock-create retry budget at the bottom of `acquire_lock` so the
        // JSON write+rename path absorbs the same transient family that
        // closed Sentry OPENHUMAN-TAURI-H1 / H8 for the lock path. Outer
        // `with_context` preserved so the Sentry fingerprint shape is stable
        // across releases. (Sentry TAURI-RUST-92J / #3355.)
        retry_with_backoff(
            "write auth profile tmp",
            PERSIST_RETRY_ATTEMPTS,
            PERSIST_RETRY_BASE_MS,
            || {
                self.consume_test_transient_failure_write()?;
                write_owner_only(&tmp_path, &json).context("write auth profile tmp")
            },
        )
        .with_context(|| {
            format!(
                "Failed to write temporary auth profile file at {}",
                tmp_path.display()
            )
        })?;

        let rename_result = retry_with_backoff(
            "replace auth profile store",
            PERSIST_RETRY_ATTEMPTS,
            PERSIST_RETRY_BASE_MS,
            || {
                self.consume_test_transient_failure_rename()?;
                fs::rename(&tmp_path, &self.path).context("rename auth profile tmp -> store")
            },
        )
        .with_context(|| {
            format!(
                "Failed to replace auth profile store at {}",
                self.path.display()
            )
        });

        if rename_result.is_err() {
            // Best-effort orphan cleanup: `tmp_path` is `…tmp.{pid}.{nanos}`
            // — unique per call — so a permanently-failing rename otherwise
            // leaks one tmp file per `app_state_snapshot` poll (~2s cadence)
            // under sustained Windows AV / Search-Indexer holds. Cleaning
            // here keeps the directory tidy; the cleanup itself can fail
            // (the same AV that blocked the rename may block the unlink),
            // which is why we deliberately drop the result.
            let _ = fs::remove_file(&tmp_path);
        }

        rename_result
    }

    /// Consume one test-injected transient FS failure for the **write**
    /// stage if any are queued. No-op in production builds.
    #[cfg(test)]
    fn consume_test_transient_failure_write(&self) -> Result<()> {
        consume_one(&self.force_transient_failures_write)
    }

    /// Consume one test-injected transient FS failure for the **rename**
    /// stage if any are queued. No-op in production builds.
    #[cfg(test)]
    fn consume_test_transient_failure_rename(&self) -> Result<()> {
        consume_one(&self.force_transient_failures_rename)
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn consume_test_transient_failure_write(&self) -> Result<()> {
        Ok(())
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn consume_test_transient_failure_rename(&self) -> Result<()> {
        Ok(())
    }

    /// Queue `n` test-only forced transient FS failures for the write
    /// stage. The next `n` calls inside the `fs::write(tmp)` retry loop
    /// return a `__TEST_TRANSIENT__` error before the underlying FS op
    /// runs; the retry helper treats them as retryable.
    #[cfg(test)]
    pub(super) fn force_next_write_failures(&self, n: usize) {
        self.force_transient_failures_write
            .store(n, Ordering::SeqCst);
    }

    /// Queue `n` test-only forced transient FS failures for the rename
    /// stage. Separate from the write counter so tests can exercise the
    /// rename retry loop in isolation (PR #3364 review feedback).
    #[cfg(test)]
    pub(super) fn force_next_rename_failures(&self, n: usize) {
        self.force_transient_failures_rename
            .store(n, Ordering::SeqCst);
    }

    /// Test introspection: how many forced write-stage failures are still
    /// queued.
    #[cfg(test)]
    pub(super) fn remaining_forced_write_failures(&self) -> usize {
        self.force_transient_failures_write.load(Ordering::SeqCst)
    }

    /// Test introspection: how many forced rename-stage failures are still
    /// queued.
    #[cfg(test)]
    pub(super) fn remaining_forced_rename_failures(&self) -> usize {
        self.force_transient_failures_rename.load(Ordering::SeqCst)
    }

    /// Queue a single test-only forced `StorageFull` lock-create failure. The
    /// next `acquire_lock` returns the synthetic disk-full error so tests can
    /// drive the lock-free read-only fallback in [`AuthProfilesStore::load`].
    #[cfg(test)]
    pub(super) fn force_next_lock_unwritable(&self) {
        self.force_lock_unwritable.store(true, Ordering::SeqCst);
    }

    fn encrypt_optional(&self, value: Option<&str>) -> Result<Option<String>> {
        match value {
            Some(value) if !value.is_empty() => self.secret_store.encrypt(value).map(Some),
            Some(_) | None => Ok(None),
        }
    }

    fn decrypt_optional(&self, value: Option<&str>) -> Result<(Option<String>, Option<String>)> {
        match value {
            Some(value) if !value.is_empty() => {
                let (plaintext, migrated) = self.secret_store.decrypt_and_migrate(value)?;
                Ok((Some(plaintext), migrated))
            }
            Some(_) | None => Ok((None, None)),
        }
    }

    fn acquire_lock(&self) -> Result<AuthProfileLockGuard> {
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
        let in_process_guard = loop {
            match in_process_lock.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(poisoned)) => break poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    if started_at.elapsed().as_millis() as u64 >= LOCK_TIMEOUT_MS {
                        anyhow::bail!("Timed out waiting for auth profile lock");
                    }
                    thread::sleep(Duration::from_millis(LOCK_WAIT_MS));
                }
            }
        };

        let mut cleared_stale = false;
        // Periodically re-probe for stale locks during the busy-wait. A
        // lock that started fresh (live pid, recent mtime) can age past
        // STALE_LOCK_AGE_MS while we wait, and we want to recover from
        // that without bailing at the LOCK_TIMEOUT_MS boundary.
        let mut next_stale_recheck_ms: u64 = 1_000;
        loop {
            let open_result = crate::openhuman::util::retry_with_backoff(
                "create auth profile lock",
                6,
                100,
                || {
                    OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&self.lock_path)
                        .context("open lock file")
                },
            );

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
    fn clear_lock_if_stale(&self) -> bool {
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
}
