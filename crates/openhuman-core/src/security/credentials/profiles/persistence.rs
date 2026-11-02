//! Reading and atomically writing the persisted auth-profile JSON store,
//! plus secret encryption for the keychain-unavailable fallback path.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fs;

#[cfg(test)]
use std::sync::atomic::Ordering;

#[cfg(test)]
use super::consume_one;
use super::{
    profile_kind_to_string, quarantine_corrupt_store, retry_with_backoff, write_owner_only,
    AuthProfile, AuthProfileKind, AuthProfilesData, AuthProfilesStore, EncryptedProfileFields,
    PersistedAuthProfile, PersistedAuthProfiles, CURRENT_SCHEMA_VERSION, PERSIST_RETRY_ATTEMPTS,
    PERSIST_RETRY_BASE_MS, PROFILES_FILENAME,
};

/// The secret the profiles live in on a storage backend
/// ([`crate::storage::secrets`]): the same JSON as `auth-profiles.json`,
/// encrypted as one secret in the acting agent's scope.
pub(crate) const STORAGE_SECRET_NAME: &str = "file:auth-profiles.json";

impl AuthProfilesStore {
    pub(super) fn save_locked(&self, data: &AuthProfilesData) -> Result<()> {
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

        if !self.use_keychain {
            // Entries whose secrets are only in the unavailable keychain never
            // reach `data`; keep them so they resolve once it reads again.
            for (id, held) in self.read_persisted_locked()?.profiles {
                if !persisted.profiles.contains_key(&id)
                    && super::migration::keychain_held_oauth(&held)
                {
                    persisted.profiles.insert(id, held);
                }
            }
        }

        self.write_persisted_locked(&persisted)
    }

    /// Encrypt a profile's secret fields for JSON storage (keychain-unavailable path).
    pub(super) fn encrypt_for_json(&self, profile: &AuthProfile) -> Result<EncryptedProfileFields> {
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

    pub(super) fn read_persisted_locked(&self) -> Result<PersistedAuthProfiles> {
        if let Some(secrets) = crate::storage::secrets::current()? {
            let Some(bytes) = crate::storage::secrets::get_blocking(&secrets, STORAGE_SECRET_NAME)?
            else {
                // First use of the backend: adopt the profiles the on-disk
                // store holds, so they do not vanish from reads and the next
                // save cannot replace them with an empty set. The file is left
                // in place (never deleted by this path).
                let legacy = self.read_file_locked()?;
                if self.path.exists() && !legacy.profiles.is_empty() {
                    let json = serde_json::to_vec(&legacy)
                        .context("Failed to serialize migrated auth profiles")?;
                    // A failed adoption keeps the legacy profiles readable;
                    // the next read retries it.
                    match crate::storage::secrets::set_blocking(
                        &secrets,
                        STORAGE_SECRET_NAME,
                        &json,
                    ) {
                        Ok(()) => tracing::info!(
                            "[credentials] migrated auth profiles file to storage secret"
                        ),
                        Err(_) => tracing::warn!(
                            "[credentials] auth profiles migration to storage failed; will retry"
                        ),
                    }
                }
                return Ok(legacy);
            };
            // No quarantine on a storage backend: an unparseable record is an
            // error, so a later write can never replace profiles it could not
            // read.
            let mut persisted: PersistedAuthProfiles = serde_json::from_slice(&bytes)
                .context("auth profile record on the storage backend is not valid JSON")?;
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
            return Ok(persisted);
        }
        self.read_file_locked()
    }

    /// Reads the on-disk `auth-profiles.json` store (empty when absent).
    fn read_file_locked(&self) -> Result<PersistedAuthProfiles> {
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

    pub(super) fn write_persisted_locked(&self, persisted: &PersistedAuthProfiles) -> Result<()> {
        if let Some(secrets) = crate::storage::secrets::current()? {
            let json =
                serde_json::to_vec(persisted).context("Failed to serialize auth profiles")?;
            crate::storage::secrets::set_blocking(&secrets, STORAGE_SECRET_NAME, &json)?;
            return Ok(());
        }
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

    pub(super) fn encrypt_optional(&self, value: Option<&str>) -> Result<Option<String>> {
        match value {
            Some(value) if !value.is_empty() => self.secret_store.encrypt(value).map(Some),
            Some(_) | None => Ok(None),
        }
    }

    pub(super) fn decrypt_optional(
        &self,
        value: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        match value {
            Some(value) if !value.is_empty() => {
                let (plaintext, migrated) = self.secret_store.decrypt_and_migrate(value)?;
                Ok((Some(plaintext), migrated))
            }
            Some(_) | None => Ok((None, None)),
        }
    }
}
