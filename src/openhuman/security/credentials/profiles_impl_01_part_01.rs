
impl AuthProfilesStore {

    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        let user_id = user_id_from_state_dir(state_dir);
        let policy = crate::openhuman::security::keyring_consent::policy::check_secret_access();
        let use_keychain = policy
            == crate::openhuman::security::keyring_consent::PolicyDecision::Proceed
            && crate::openhuman::security::keyring::is_available();
        log::debug!(
            "[auth] AuthProfilesStore::new state_dir={} user_id={user_id} use_keychain={use_keychain} policy={policy:?}",
            state_dir.display()
        );
        match policy {
            crate::openhuman::security::keyring_consent::PolicyDecision::Proceed => {
                if !use_keychain {
                    // OS keychain unavailable despite Proceed policy (probe failed).
                    log::info!(
                        "[auth] OS keychain unavailable — using encrypted JSON for auth profiles user_id={user_id}"
                    );
                }
            }
            crate::openhuman::security::keyring_consent::PolicyDecision::ConsentRequired => {
                log::warn!(
                    "[auth] keyring consent has not been given — secrets will NOT be persisted \
                     to the OS keychain until the user grants consent. \
                     Falling back to encrypted JSON for auth profiles user_id={user_id}"
                );
            }
            crate::openhuman::security::keyring_consent::PolicyDecision::Declined => {
                log::warn!(
                    "[auth] user explicitly declined OS keychain storage — \
                     using encrypted JSON for auth profiles user_id={user_id}"
                );
            }
        }
        Self {
            path: state_dir.join(PROFILES_FILENAME),
            lock_path: state_dir.join(LOCK_FILENAME),
            secret_store: SecretStore::new(state_dir, encrypt_secrets),
            user_id,
            use_keychain,
            #[cfg(test)]
            force_transient_failures_write: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            force_transient_failures_rename: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            force_lock_unwritable: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a keychain key for an auth profile's combined secret payload.
    fn keychain_key_for_profile(&self, profile_id: &str) -> String {
        format!("{KEYCHAIN_AUTH_PREFIX}{profile_id}")
    }

    /// Store auth secrets for a profile in the OS keychain.
    ///
    /// The secrets are serialized as a compact JSON object so a single
    /// keychain entry holds all token fields for the profile.
    fn keychain_store_secrets(&self, profile: &AuthProfile) -> anyhow::Result<()> {
        let key = self.keychain_key_for_profile(&profile.id);
        let secrets = serde_json::json!({
            "token": profile.token,
            "access_token": profile.token_set.as_ref().map(|ts| &ts.access_token),
            "refresh_token": profile.token_set.as_ref().and_then(|ts| ts.refresh_token.as_deref()),
            "id_token": profile.token_set.as_ref().and_then(|ts| ts.id_token.as_deref()),
        });
        let payload = serde_json::to_string(&secrets)
            .context("Failed to serialize auth secrets for keychain")?;
        crate::openhuman::security::keyring::set(&self.user_id, &key, &payload).map_err(|e| {
            anyhow::anyhow!(
                "Keychain set failed for profile {}: {e} | detail={}",
                profile.id,
                e.diagnostic()
            )
        })?;
        log::debug!(
            "[auth] keychain_store_secrets stored profile_id={} user_id={}",
            profile.id,
            self.user_id
        );
        Ok(())
    }

    /// Load auth secrets for a profile from the OS keychain.
    ///
    /// Returns `None` if no keychain entry exists for the profile.
    fn keychain_load_secrets(&self, profile_id: &str) -> anyhow::Result<Option<KeychainSecrets>> {
        let key = self.keychain_key_for_profile(profile_id);
        let payload = match crate::openhuman::security::keyring::get(&self.user_id, &key) {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::debug!(
                    "[auth] keychain_load_secrets miss profile_id={profile_id} user_id={}",
                    self.user_id
                );
                return Ok(None);
            }
            Err(e) => {
                log::warn!(
                    "[auth] keychain_load_secrets error profile_id={profile_id} user_id={}: {e} | detail={}",
                    self.user_id,
                    e.diagnostic()
                );
                return Ok(None);
            }
        };
        let secrets: KeychainSecrets = serde_json::from_str(&payload).map_err(|e| {
            anyhow::anyhow!("Keychain payload for profile {profile_id} is not valid JSON: {e}")
        })?;
        log::debug!(
            "[auth] keychain_load_secrets hit profile_id={profile_id} user_id={}",
            self.user_id
        );
        Ok(Some(secrets))
    }

    /// Delete keychain secrets for a profile (called on profile removal).
    fn keychain_delete_secrets(&self, profile_id: &str) {
        let key = self.keychain_key_for_profile(profile_id);
        if let Err(e) = crate::openhuman::security::keyring::delete(&self.user_id, &key) {
            log::warn!(
                "[auth] keychain_delete_secrets error profile_id={profile_id} user_id={}: {e} | detail={}",
                self.user_id,
                e.diagnostic()
            );
        } else {
            log::debug!(
                "[auth] keychain_delete_secrets ok profile_id={profile_id} user_id={}",
                self.user_id
            );
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AuthProfilesData> {
        match self.acquire_lock() {
            Ok(_lock) => self.load_locked(),
            Err(e) if is_lock_create_unwritable_fs(&e) => {
                // RCA Sentry TAURI-RUST-4SZ: a full / read-only filesystem
                // can't create the exclusive lock file, but the store already
                // exists and writers publish via atomic tmp+rename, so a
                // lock-free read is still consistent. The read path is the
                // hot caller here (`app_state_snapshot` polls it every tick),
                // so failing it strands the UI AND floods Sentry once per
                // poll. Degrade to a lock-free read-only load instead — the
                // user keeps their session view, and because no error is
                // produced the noise stops at the source rather than being
                // suppressed downstream. Opportunistic migrations are skipped
                // (they couldn't persist on a full disk anyway).
                log::warn!(
                    "[auth] auth-profile lock could not be created ({e}); \
                     serving lock-free read-only load (likely disk full / read-only FS)"
                );
                self.load_unlocked_readonly()
            }
            Err(e) => Err(e),
        }
    }

    pub fn upsert_profile(&self, mut profile: AuthProfile, set_active: bool) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        profile.updated_at = Utc::now();
        if let Some(existing) = data.profiles.get(&profile.id) {
            profile.created_at = existing.created_at;
        }

        if set_active {
            data.active_profiles
                .insert(profile.provider.clone(), profile.id.clone());
        }

        data.profiles.insert(profile.id.clone(), profile);
        data.updated_at = Utc::now();

        self.save_locked(&data)
    }

    pub fn remove_profile(&self, profile_id: &str) -> Result<bool> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        let removed = data.profiles.remove(profile_id).is_some();
        if !removed {
            return Ok(false);
        }

        data.active_profiles
            .retain(|_, active| active != profile_id);
        data.updated_at = Utc::now();
        self.save_locked(&data)?;

        // Clean up keychain entry for this profile (idempotent if keychain
        // is unavailable or no entry exists).
        if self.use_keychain {
            self.keychain_delete_secrets(profile_id);
        }

        Ok(true)
    }

    pub fn set_active_profile(&self, provider: &str, profile_id: &str) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        if !data.profiles.contains_key(profile_id) {
            anyhow::bail!("Auth profile not found: {profile_id}");
        }

        data.active_profiles
            .insert(provider.to_string(), profile_id.to_string());
        data.updated_at = Utc::now();
        self.save_locked(&data)
    }

    pub fn clear_active_profile(&self, provider: &str) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;
        data.active_profiles.remove(provider);
        data.updated_at = Utc::now();
        self.save_locked(&data)
    }

    pub fn update_profile<F>(&self, profile_id: &str, mut updater: F) -> Result<AuthProfile>
    where
        F: FnMut(&mut AuthProfile) -> Result<()>,
    {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        let profile = data
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| anyhow::anyhow!("Auth profile not found: {profile_id}"))?;

        updater(profile)?;
        profile.updated_at = Utc::now();
        let updated_profile = profile.clone();
        data.updated_at = Utc::now();
        self.save_locked(&data)?;
        Ok(updated_profile)
    }

    fn load_locked(&self) -> Result<AuthProfilesData> {
        self.load_resolved(true)
    }

    /// Lock-free read-only load used as the [`AuthProfilesStore::load`]
    /// fallback when the exclusive lock can't be created because the
    /// filesystem won't accept the lock file (disk full / read-only mount —
    /// Sentry TAURI-RUST-4SZ). Safe without the lock because writers publish
    /// the store atomically (tmp + `fs::rename`), so a bare read always sees
    /// a complete file. Skips the opportunistic migration / dropped-profile
    /// rewrite that `load_locked` performs — that write needs both the lock
    /// and a writable disk, and this path runs precisely when neither holds.
    fn load_unlocked_readonly(&self) -> Result<AuthProfilesData> {
        self.load_resolved(false)
    }

    /// Shared read + in-memory resolution worker. Reads the persisted store,
    /// resolves/migrates secrets and drops unrecoverable profiles in memory,
    /// and — only when `persist` is true — writes back any resulting cleanup.
    /// The returned `AuthProfilesData` reflects the in-memory cleanup either
    /// way, so the lock-free read path (`persist = false`) still returns a
    /// correct, fully-resolved view without touching disk.
    fn load_resolved(&self, persist: bool) -> Result<AuthProfilesData> {
        let mut persisted = self.read_persisted_locked()?;
        // `migrated` tracks enc: → enc2: XOR-cipher upgrades (original behavior).
        let mut migrated = false;
        // `keychain_migrated` tracks enc2: → keychain promotions: when true the
        // persisted JSON must be rewritten with secret fields cleared.
        let mut keychain_migrated = false;
        let mut dropped_ids: Vec<String> = Vec::new();

        let mut profiles = BTreeMap::new();
        for (id, p) in &mut persisted.profiles {
            // ── Step 1: Resolve secrets ───────────────────────────────────────
            //
            // Priority order:
            //   (a) OS keychain — preferred when available.
            //   (b) enc2:/enc: JSON fields — legacy; decrypt and optionally
            //       migrate to keychain on this read.
            //   (c) Plaintext JSON fields — oldest legacy path; pass through.
            //
            // A decrypt failure (wrong key / tampered data) drops the profile
            // rather than poisoning every reader — the user falls back to a
            // clean logged-out state and re-authenticates cleanly.

            let (access_token, refresh_token, id_token, token) = if self.use_keychain {
                // ── (a) Keychain path ──────────────────────────────────────
                match self.keychain_load_secrets(id) {
                    Ok(Some(secrets)) => {
                        // Keychain has the entry — use it directly.  Clear the
                        // JSON secret fields so they're wiped on next save.
                        let had_enc_fields = p.access_token.is_some()
                            || p.refresh_token.is_some()
                            || p.id_token.is_some()
                            || p.token.is_some();
                        if had_enc_fields {
                            log::info!(
                                "[auth] load: clearing legacy enc fields for profile_id={id} (already in keychain)"
                            );
                            p.access_token = None;
                            p.refresh_token = None;
                            p.id_token = None;
                            p.token = None;
                            keychain_migrated = true;
                        }
                        (
                            secrets.access_token,
                            secrets.refresh_token,
                            secrets.id_token,
                            secrets.token,
                        )
                    }
                    Ok(None) => {
                        // ── (b) No keychain entry yet — decrypt JSON fields and migrate ──
                        let decrypted = (|| -> Result<_> {
                            let (access_token, access_mig) =
                                self.decrypt_optional(p.access_token.as_deref())?;
                            let (refresh_token, refresh_mig) =
                                self.decrypt_optional(p.refresh_token.as_deref())?;
                            let (id_token, id_mig) =
                                self.decrypt_optional(p.id_token.as_deref())?;
                            let (token, token_mig) = self.decrypt_optional(p.token.as_deref())?;
                            Ok((
                                access_token,
                                access_mig,
                                refresh_token,
                                refresh_mig,
                                id_token,
                                id_mig,
                                token,
                                token_mig,
                            ))
                        })();
                        let (at, at_mig, rt, rt_mig, it, it_mig, tok, tok_mig) = match decrypted {
                            Ok(v) => v,
                            Err(e) => {
                                log::warn!(
                                    "[auth] dropping unrecoverable profile provider={}: {e}. \
                                         Most likely cause: .secret_key was regenerated. \
                                         Re-authenticate to restore the session.",
                                    p.provider
                                );
                                dropped_ids.push(id.clone());
                                continue;
                            }
                        };
                        // Track XOR→enc2 cipher upgrades (existing behavior).
                        if at_mig.is_some() {
                            p.access_token = at_mig;
                            migrated = true;
                        }
                        if rt_mig.is_some() {
                            p.refresh_token = rt_mig;
                            migrated = true;
                        }
                        if it_mig.is_some() {
                            p.id_token = it_mig;
                            migrated = true;
                        }
                        if tok_mig.is_some() {
                            p.token = tok_mig;
                            migrated = true;
                        }

                        // If any secrets were found in JSON, promote them to keychain
                        // and clear the JSON fields so the next write is clean.
                        let has_secrets =
                            at.is_some() || rt.is_some() || it.is_some() || tok.is_some();
                        if has_secrets {
                            log::info!(
                                "[auth] load: migrating enc fields to keychain profile_id={id} user_id={}",
                                self.user_id
                            );
                            let dummy_profile = AuthProfile {
                                id: id.clone(),
                                provider: p.provider.clone(),
                                profile_name: p.profile_name.clone(),
                                kind: parse_profile_kind(&p.kind).unwrap_or(AuthProfileKind::Token),
                                account_id: p.account_id.clone(),
                                workspace_id: p.workspace_id.clone(),
                                token_set: at.clone().map(|access| TokenSet {
                                    access_token: access,
                                    refresh_token: rt.clone(),
                                    id_token: it.clone(),
                                    expires_at: None,
                                    token_type: None,
                                    scope: None,
                                }),
                                token: tok.clone(),
                                metadata: Default::default(),
                                created_at: Utc::now(),
                                updated_at: Utc::now(),
                            };
                            if let Err(e) = self.keychain_store_secrets(&dummy_profile) {
                                // Non-fatal: keep the enc2: fields in JSON so the
                                // next load can try again.
                                log::warn!(
                                    "[auth] load: keychain migration failed profile_id={id}: {e}; \
                                     keeping enc fields in JSON"
                                );
                            } else {
                                // Wipe JSON secret fields now that keychain has them.
                                p.access_token = None;
                                p.refresh_token = None;
                                p.id_token = None;
                                p.token = None;
                                keychain_migrated = true;
                            }
                        }
                        (at, rt, it, tok)
                    }
                    Err(_e) => {
                        // Keychain I/O error — fall through to JSON decrypt path.
                        log::warn!(
                            "[auth] keychain error for profile_id={id}; falling back to JSON"
                        );
                        let decrypted = (|| -> Result<_> {
                            let (at, _) = self.decrypt_optional(p.access_token.as_deref())?;
                            let (rt, _) = self.decrypt_optional(p.refresh_token.as_deref())?;
                            let (it, _) = self.decrypt_optional(p.id_token.as_deref())?;
                            let (tok, _) = self.decrypt_optional(p.token.as_deref())?;
                            Ok((at, rt, it, tok))
                        })();
                        match decrypted {
                            Ok(v) => v,
                            Err(e) => {
                                log::warn!(
                                    "[auth] dropping unrecoverable profile provider={}: {e}",
                                    p.provider
                                );
                                dropped_ids.push(id.clone());
                                continue;
                            }
                        }
                    }
                }
            } else {
                // ── (b/c) No keychain — use existing JSON decrypt path ────────
                let decrypted = (|| -> Result<_> {
                    let (access_token, access_migrated) =
                        self.decrypt_optional(p.access_token.as_deref())?;
                    let (refresh_token, refresh_migrated) =
                        self.decrypt_optional(p.refresh_token.as_deref())?;
                    let (id_token, id_migrated) = self.decrypt_optional(p.id_token.as_deref())?;
                    let (token, token_migrated) = self.decrypt_optional(p.token.as_deref())?;
                    Ok((
                        access_token,
                        access_migrated,
                        refresh_token,
                        refresh_migrated,
                        id_token,
                        id_migrated,
                        token,
                        token_migrated,
                    ))
                })();

                let (
                    access_token,
                    access_migrated,
                    refresh_token,
                    refresh_migrated,
                    id_token,
                    id_migrated,
                    token,
                    token_migrated,
                ) = match decrypted {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!(
                            "[auth] dropping unrecoverable profile provider={}: {e}. \
                             Most likely cause: .secret_key was regenerated after this profile \
                             was stored. The store will be rewritten without this entry; \
                             re-authenticate to restore the session.",
                            p.provider
                        );
                        dropped_ids.push(id.clone());
                        continue;
                    }
                };

                if let Some(value) = access_migrated {
                    p.access_token = Some(value);
                    migrated = true;
                }
                if let Some(value) = refresh_migrated {
                    p.refresh_token = Some(value);
                    migrated = true;
                }
                if let Some(value) = id_migrated {
                    p.id_token = Some(value);
                    migrated = true;
                }
                if let Some(value) = token_migrated {
                    p.token = Some(value);
                    migrated = true;
                }
                (access_token, refresh_token, id_token, token)
            };

            let kind = match parse_profile_kind(&p.kind) {
                Ok(k) => k,
                Err(e) => {
                    // A single profile with an unrecognized `kind` (e.g. a legacy value
                    // like "OAuth" written before the kebab-case rename, or "api_key"
                    // written by an older code path) must not poison the whole store —
                    // otherwise every reader fails the entire load and the user is
                    // locked out of *all* their auth profiles. Drop just this entry,
                    // matching the decrypt-failure recovery pattern above; the next
                    // login re-encodes the kind correctly.
                    log::warn!(
                        "[auth] dropping profile with unrecognized kind={:?} provider={}: {e}. \
                         This usually means the profile was written by an older version of \
                         OpenHuman. Re-authenticate to restore the session.",
                        p.kind,
                        p.provider
                    );
                    dropped_ids.push(id.clone());
                    continue;
                }
            };
            let token_set = match kind {
                AuthProfileKind::OAuth => {
                    let access = match access_token {
                        Some(a) => a,
                        None => {
                            log::warn!(
                                "[auth] dropping OAuth profile with missing access_token: \
                                 provider={}. Re-authenticate to restore.",
                                p.provider
                            );
                            dropped_ids.push(id.clone());
                            continue;
                        }
                    };
                    Some(TokenSet {
                        access_token: access,
                        refresh_token,
                        id_token,
                        expires_at: parse_optional_datetime(p.expires_at.as_deref())?,
                        token_type: p.token_type.clone(),
                        scope: p.scope.clone(),
                    })
                }
                AuthProfileKind::Token => None,
            };

            profiles.insert(
                id.clone(),
                AuthProfile {
                    id: id.clone(),
                    provider: p.provider.clone(),
                    profile_name: p.profile_name.clone(),
                    kind,
                    account_id: p.account_id.clone(),
                    workspace_id: p.workspace_id.clone(),
                    token_set,
                    token,
                    metadata: p.metadata.clone(),
                    created_at: parse_datetime_with_fallback(&p.created_at),
                    updated_at: parse_datetime_with_fallback(&p.updated_at),
                },
            );
        }

        // Purge dropped profiles from the on-disk persisted view AND
        // any `active_profiles` pointers that referenced them, so the
        // next read returns a clean "no active session" state.
        if !dropped_ids.is_empty() {
            // Always apply the cleanup to the in-memory view so the returned
            // data is correct even on the lock-free read path; the on-disk
            // rewrite below is what's gated by `persist`.
            for id in &dropped_ids {
                persisted.profiles.remove(id);
            }
            persisted
                .active_profiles
                .retain(|_, profile_id| !dropped_ids.contains(profile_id));
            persisted.updated_at = Utc::now().to_rfc3339();
            log::warn!(
                "[auth] purged {} unrecoverable profile(s) from store at {} \
                 (provider list redacted to avoid leaking PII)",
                dropped_ids.len(),
                self.path.display(),
            );
        }
        // Persist opportunistic cleanup / migrations only on the locked write
        // path. The lock-free read-only fallback (`persist = false`, used when
        // the disk can't accept the lock file) intentionally skips this — the
        // write would fail on a full disk anyway, and the in-memory view above
        // is already correct.
        if persist && (!dropped_ids.is_empty() || migrated || keychain_migrated) {
            self.write_persisted_locked(&persisted)?;
        }

        Ok(AuthProfilesData {
            schema_version: persisted.schema_version,
            updated_at: parse_datetime_with_fallback(&persisted.updated_at),
            active_profiles: persisted.active_profiles,
            profiles,
        })
    }
}
