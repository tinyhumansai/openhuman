use crate::openhuman::keyring::SecretStore;
use crate::openhuman::util::retry_with_backoff;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::Arc;

const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Compact secret payload stored as a single keychain entry per auth profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeychainSecrets {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
}
const PROFILES_FILENAME: &str = "auth-profiles.json";
const LOCK_FILENAME: &str = "auth-profiles.lock";
const LOCK_WAIT_MS: u64 = 50;
/// A lock file that has existed for longer than this is treated as leaked
/// (its owner crashed without unlinking it, or `fs::remove_file` in the
/// guard's `Drop` was rejected by Windows AV/indexer and the file got
/// orphaned with the still-alive owner's pid in it). No legitimate
/// auth-profile operation holds the lock for anywhere near this long —
/// load+save is a tiny JSON read followed by an atomic rename. The
/// threshold is intentionally well above any realistic operation time
/// so we never reclaim under a slow-but-legitimate holder.
const STALE_LOCK_AGE_MS: u64 = 30_000;
/// Staleness threshold for a **malformed** lock — one with no parseable
/// `pid=` line. A healthy holder writes its pid microseconds after the
/// `create_new` succeeds, so a pidless lock older than this can only be a
/// crash/kill that landed between `create_new` and the `pid=` write (or an
/// abandoned in-flight writer). It is never a live, well-behaved holder, so
/// we reclaim it after a short grace instead of making every reader wait the
/// full [`STALE_LOCK_AGE_MS`]. This is what was leaving users stuck on
/// "Initializing OpenHuman" for ~30s after a kill+reopen: `app_state_snapshot`
/// → `load_app_session_profile` → `acquire_lock` blocked on a fresh pidless
/// lock. The grace is generous enough to never reclaim under a live writer
/// mid-`create_new`/`pid=` window (microseconds in practice).
const MALFORMED_LOCK_GRACE_MS: u64 = 2_000;
/// Wait long enough for a fresh leaked lock to cross the stale threshold
/// and be reclaimed before surfacing a lock timeout to the caller.
const LOCK_TIMEOUT_MS: u64 = STALE_LOCK_AGE_MS + 5_000;

/// Retry budget for the JSON write + rename in `write_persisted_locked`.
/// Same shape as the lock-create call at the bottom of `acquire_lock` (which
/// is what closed Sentry OPENHUMAN-TAURI-H1 / H8 in #1641 / #2085). With
/// `attempts = 6`, `retry_with_backoff` issues at most 6 calls and sleeps
/// 5 times between them (last failure breaks without sleeping):
/// `100+200+400+800+1600 ≈ 3.1s per stage`, so the write and rename stages
/// together sit at `≈6.2s` worst case. Sized to stay well inside
/// `LOCK_TIMEOUT_MS = 35_000` so concurrent acquire_lock callers never time
/// out behind a single retry-loop owner.
const PERSIST_RETRY_ATTEMPTS: u32 = 6;
const PERSIST_RETRY_BASE_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthProfileKind {
    OAuth,
    Token,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl TokenSet {
    pub fn is_expiring_within(&self, skew: Duration) -> bool {
        match self.expires_at {
            Some(expires_at) => {
                let now_plus_skew =
                    Utc::now() + chrono::Duration::from_std(skew).unwrap_or_default();
                expires_at <= now_plus_skew
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    pub id: String,
    pub provider: String,
    pub profile_name: String,
    pub kind: AuthProfileKind,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub token_set: Option<TokenSet>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AuthProfile {
    pub fn new_oauth(provider: &str, profile_name: &str, token_set: TokenSet) -> Self {
        let now = Utc::now();
        let id = profile_id(provider, profile_name);
        Self {
            id,
            provider: provider.to_string(),
            profile_name: profile_name.to_string(),
            kind: AuthProfileKind::OAuth,
            account_id: None,
            workspace_id: None,
            token_set: Some(token_set),
            token: None,
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_token(provider: &str, profile_name: &str, token: String) -> Self {
        let now = Utc::now();
        let id = profile_id(provider, profile_name);
        Self {
            id,
            provider: provider.to_string(),
            profile_name: profile_name.to_string(),
            kind: AuthProfileKind::Token,
            account_id: None,
            workspace_id: None,
            token_set: None,
            token: Some(token),
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfilesData {
    pub schema_version: u32,
    pub updated_at: DateTime<Utc>,
    pub active_profiles: BTreeMap<String, String>,
    pub profiles: BTreeMap<String, AuthProfile>,
}

impl Default for AuthProfilesData {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: Utc::now(),
            active_profiles: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
    }
}

/// Prefix used for keychain entries that store auth profile secrets.
/// Full key format (as handled by the keyring module): `"{user_id}:auth:{profile_id}"`.
const KEYCHAIN_AUTH_PREFIX: &str = "auth:";

/// Derive a stable keychain user-id from a state directory path.
///
/// For a typical path like `/home/alice/.openhuman/users/uid-123` this
/// returns `"uid-123"`.  Falls back to a hash of the full path string so
/// the function always returns a non-empty value even for unusual layouts.
fn user_id_from_state_dir(state_dir: &Path) -> String {
    // The user directory is `{root}/users/{user_id}/` — take the last component.
    if let Some(id) = state_dir.file_name().and_then(|s| s.to_str()) {
        if !id.is_empty() {
            return id.to_string();
        }
    }
    // Fallback: use a hex hash of the path so we always get a stable string.
    let path_str = state_dir.to_string_lossy();
    let mut hash: u64 = 14695981039346656037u64; // FNV-1a offset basis
    for b in path_str.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(1099511628211u64);
    }
    format!("path-{hash:016x}")
}

#[derive(Debug, Clone)]
pub struct AuthProfilesStore {
    path: PathBuf,
    lock_path: PathBuf,
    secret_store: SecretStore,
    /// Opaque user identifier used to namespace keychain entries.
    user_id: String,
    /// Whether the OS keychain is available on this machine.
    /// Cached at construction time to avoid repeated probes.
    use_keychain: bool,
    /// `#[cfg(test)]` failure injection for the **write** stage of
    /// `write_persisted_locked`. When non-zero, the next call inside the
    /// `fs::write(tmp)` retry loop consumes one count and returns a
    /// `__TEST_TRANSIENT__` error so `is_transient_fs_error` treats it as
    /// retryable (`src/openhuman/util.rs:618`). Production binaries never
    /// see this field.
    #[cfg(test)]
    force_transient_failures_write: Arc<AtomicUsize>,
    /// `#[cfg(test)]` failure injection for the **rename** stage of
    /// `write_persisted_locked`. Separate counter from the write stage so a
    /// test can exercise the rename retry loop without first having to drain
    /// failures through the write stage (see PR #3364 review feedback —
    /// the headline retry path was line-covered but not behaviour-covered
    /// before this split).
    #[cfg(test)]
    force_transient_failures_rename: Arc<AtomicUsize>,
    /// `#[cfg(test)]` failure injection — when set, the next `acquire_lock`
    /// call consumes the flag and returns a synthetic `StorageFull`
    /// lock-create failure, exercising the lock-free read-only fallback in
    /// [`AuthProfilesStore::load`] (Sentry TAURI-RUST-4SZ). Production
    /// binaries never see this field.
    #[cfg(test)]
    force_lock_unwritable: Arc<AtomicBool>,
}

impl AuthProfilesStore {
    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        let user_id = user_id_from_state_dir(state_dir);
        let policy = crate::openhuman::keyring_consent::policy::check_secret_access();
        let use_keychain = policy == crate::openhuman::keyring_consent::PolicyDecision::Proceed
            && crate::openhuman::keyring::is_available();
        log::debug!(
            "[auth] AuthProfilesStore::new state_dir={} user_id={user_id} use_keychain={use_keychain} policy={policy:?}",
            state_dir.display()
        );
        match policy {
            crate::openhuman::keyring_consent::PolicyDecision::Proceed => {
                if !use_keychain {
                    // OS keychain unavailable despite Proceed policy (probe failed).
                    log::info!(
                        "[auth] OS keychain unavailable — using encrypted JSON for auth profiles user_id={user_id}"
                    );
                }
            }
            crate::openhuman::keyring_consent::PolicyDecision::ConsentRequired => {
                log::warn!(
                    "[auth] keyring consent has not been given — secrets will NOT be persisted \
                     to the OS keychain until the user grants consent. \
                     Falling back to encrypted JSON for auth profiles user_id={user_id}"
                );
            }
            crate::openhuman::keyring_consent::PolicyDecision::Declined => {
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
        crate::openhuman::keyring::set(&self.user_id, &key, &payload).map_err(|e| {
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
        let payload = match crate::openhuman::keyring::get(&self.user_id, &key) {
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
        if let Err(e) = crate::openhuman::keyring::delete(&self.user_id, &key) {
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
    fn encrypt_for_json(
        &self,
        profile: &AuthProfile,
    ) -> Result<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> {
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
                fs::write(&tmp_path, &json).context("write auth profile tmp")
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
                        .map_or(false, |ioe| ioe.kind() == std::io::ErrorKind::AlreadyExists);

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
        let too_old = age.map_or(false, |a| a >= Duration::from_millis(STALE_LOCK_AGE_MS));
        // A pidless lock needs only a short grace: no healthy holder leaves the
        // file without a `pid=` line for more than the microsecond gap between
        // `create_new` and the write, so anything older is abandoned. If mtime
        // is unreadable (clock skew, platform limitation) default to stale —
        // no legitimate in-flight writer would be undetectable for that long.
        let malformed_too_old = age.map_or(true, |a| {
            a >= Duration::from_millis(MALFORMED_LOCK_GRACE_MS)
        });

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

/// Cross-platform best-effort check that a given OS process id is currently
/// running. Used by [`AuthProfilesStore::clear_lock_if_stale`] to decide
/// whether a recorded lock owner is still alive; a false negative just
/// means we keep waiting on a lock that was actually already gone, which
/// is the safe direction. Backed by sysinfo so we don't grow a new libc /
/// windows-sys dependency for one syscall.
/// Wrap a non-`AlreadyExists` `create_new` failure with a context line that
/// embeds the underlying `io::ErrorKind` and `raw_os_error()`. Pulled out
/// of [`AuthProfilesStore::acquire_lock`] so unit tests can drive the
/// formatting directly without depending on filesystem permissions (CI runs
/// as root and bypasses `chmod 0500`).
/// True when a lock-create failure was caused by the filesystem refusing to
/// accept the lock file itself — disk full (`StorageFull`, POSIX `ENOSPC` /
/// Windows `ERROR_DISK_FULL`) or a read-only mount (`ReadOnlyFilesystem`,
/// `EROFS`). These are exactly the conditions where the **read** path can
/// safely skip the exclusive lock: the store already exists, writers publish
/// atomically, and the failing operation is the *creation of a new lock file*,
/// not the read. Lock *contention* (`AlreadyExists` / the busy-wait timeout)
/// and every other error deliberately do NOT match — those still propagate so
/// genuine problems stay visible. See [`AuthProfilesStore::load`].
fn is_lock_create_unwritable_fs(err: &anyhow::Error) -> bool {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .map(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::ReadOnlyFilesystem
            )
        })
        .unwrap_or(false)
}

fn annotate_lock_create_failure(err: anyhow::Error) -> anyhow::Error {
    let io = err.chain().find_map(|c| c.downcast_ref::<std::io::Error>());
    let kind = io.map(|ioe| ioe.kind());
    let os_code = io.and_then(|ioe| ioe.raw_os_error());
    err.context(format!(
        "Failed to create auth profile lock (kind={:?}, os_code={:?})",
        kind, os_code
    ))
}

fn is_pid_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let target = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(target).is_some()
}

/// Process-wide registry of in-memory locks, one per auth-profile lock-file
/// path. Returns a `'static` handle so the held guard can live inside
/// [`AuthProfileLockGuard`] for the lifetime of the on-disk lock.
///
/// Entries are created on first use and never removed — the set of distinct
/// auth-profile lock paths in a process is tiny (effectively one), so leaking
/// a `Mutex<()>` per path is negligible and buys us a `'static` lifetime
/// without `unsafe` or a self-referential guard. Serializing same-process
/// acquirers here is what lets [`AuthProfilesStore::reclaim_self_owned_lock`]
/// treat an on-disk lock carrying our own pid as a leak (no live same-process
/// guard can exist while a caller holds this lock).
///
/// The registry key is **canonicalized** (real parent directory + lock
/// filename), so two `AuthProfilesStore`s pointing at the same lock through
/// aliased spellings (`state` vs `state/.`, relative vs absolute, a symlinked
/// parent, Windows case variants) share one mutex. Without this, aliased paths
/// would take *different* mutexes and a second same-process acquirer could see
/// the first live guard's pid, mistake it for a leak, and reclaim it — entering
/// the critical section concurrently. `acquire_lock` always calls this *after*
/// `create_dir_all(parent)`, so the parent exists and `canonicalize` succeeds;
/// if it ever fails we fall back to the raw path (no worse than before).
fn in_process_lock_for(path: &Path) -> &'static Mutex<()> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
    let key = path
        .parent()
        .and_then(|parent| {
            let canonical = fs::canonicalize(parent).ok()?;
            path.file_name().map(|name| canonical.join(name))
        })
        .unwrap_or_else(|| path.to_path_buf());
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Deref the `&mut &'static Mutex<()>` to copy the `'static` reference out,
    // so the returned handle is not tied to the dropped `map` guard.
    *map.entry(key)
        .or_insert_with(|| &*Box::leak(Box::new(Mutex::new(()))))
}

struct AuthProfileLockGuard {
    lock_path: PathBuf,
    /// Held to serialize same-process acquirers for `lock_path`; released when
    /// this guard drops. Never read — see [`in_process_lock_for`].
    _in_process: MutexGuard<'static, ()>,
}

impl Drop for AuthProfileLockGuard {
    fn drop(&mut self) {
        // Best-effort unlink with retries. On Windows, antivirus and the
        // search indexer routinely hold a transient handle on a file just
        // after it is written, which makes `fs::remove_file` fail with
        // `PermissionDenied`. A failed unlink here leaks the lock file
        // with the still-alive owner pid inside. Recovery is layered: this
        // retry loop is the first line of defence; if it still fails, the
        // next acquirer in THIS process reclaims the self-owned lock
        // immediately via `reclaim_self_owned_lock` (it holds the in-process
        // lock, so the recorded pid being ours unambiguously means a leak);
        // and the age-based reclaim in `clear_lock_if_stale` remains the
        // cross-process safety net.
        for attempt in 0..5u32 {
            match fs::remove_file(&self.lock_path) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => {
                    if attempt + 1 == 5 {
                        tracing::warn!(
                            target: "auth-profiles",
                            "[credentials] failed to remove auth profile lock at {} after {} attempts: {e}. \
                             The next acquirer in this process will reclaim the self-owned lock immediately; \
                             a leak from another process is recovered by the age-based reclaim within {}ms.",
                            self.lock_path.display(),
                            attempt + 1,
                            STALE_LOCK_AGE_MS,
                        );
                        return;
                    }
                    thread::sleep(Duration::from_millis(50u64.saturating_mul(1u64 << attempt)));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedAuthProfiles {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default = "default_now_rfc3339")]
    updated_at: String,
    #[serde(default)]
    active_profiles: BTreeMap<String, String>,
    #[serde(default)]
    profiles: BTreeMap<String, PersistedAuthProfile>,
}

impl Default for PersistedAuthProfiles {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: default_now_rfc3339(),
            active_profiles: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedAuthProfile {
    provider: String,
    profile_name: String,
    kind: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default = "default_now_rfc3339")]
    created_at: String,
    #[serde(default = "default_now_rfc3339")]
    updated_at: String,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Decrement an `AtomicUsize` failure-injection counter by one if it is
/// non-zero, returning a `__TEST_TRANSIENT__` error so `is_transient_fs_error`
/// classifies the failure as retryable. Used by both per-stage consumers in
/// `write_persisted_locked` (test-only).
#[cfg(test)]
fn consume_one(counter: &AtomicUsize) -> Result<()> {
    if counter.load(Ordering::SeqCst) == 0 {
        return Ok(());
    }
    counter.fetch_sub(1, Ordering::SeqCst);
    Err(anyhow::anyhow!(
        "__TEST_TRANSIENT__ injected transient FS failure"
    ))
}

fn parse_profile_kind(value: &str) -> Result<AuthProfileKind> {
    match value {
        "oauth" => Ok(AuthProfileKind::OAuth),
        "token" => Ok(AuthProfileKind::Token),
        other => anyhow::bail!("Unsupported auth profile kind: {other}"),
    }
}

fn profile_kind_to_string(kind: AuthProfileKind) -> &'static str {
    match kind {
        AuthProfileKind::OAuth => "oauth",
        AuthProfileKind::Token => "token",
    }
}

fn parse_optional_datetime(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value.map(parse_datetime).transpose()
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .with_context(|| format!("Invalid RFC3339 timestamp: {value}"))
}

fn parse_datetime_with_fallback(value: &str) -> DateTime<Utc> {
    parse_datetime(value).unwrap_or_else(|_| Utc::now())
}

pub fn profile_id(provider: &str, profile_name: &str) -> String {
    format!("{}:{}", provider.trim(), profile_name.trim())
}

fn quarantine_corrupt_store(path: &Path) -> Result<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("auth-profiles");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut candidate = parent.join(format!("{stem}.corrupt-{ts}.{ext}"));
    let mut suffix = 0u32;
    while candidate.exists() {
        suffix += 1;
        candidate = parent.join(format!("{stem}.corrupt-{ts}-{suffix}.{ext}"));
    }
    fs::rename(path, &candidate).with_context(|| {
        format!(
            "Failed to quarantine corrupt auth profile store {} -> {}",
            path.display(),
            candidate.display()
        )
    })?;
    Ok(candidate)
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
