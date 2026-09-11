use crate::openhuman::security::keyring::SecretStore;
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

type EncryptedProfileFields = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

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
/// Write `bytes` to `path` with the file readable only by its owner.
///
/// `fs::write` creates at `0o666 & ~umask` — `0644` under the usual `022` — and
/// `fs::rename` carries the source mode onto the destination, so the credential
/// store inherited it on every save. Any process running under a different UID
/// could read it, and on the encrypted-JSON path — the normal state on Linux and
/// on any headless install without a keyring — that file is OAuth token
/// ciphertext, leaving `.secret_key` as the only remaining control.
///
/// Same shape as the fix #2360 landed for the secret key in
/// `keyring/encrypted_store.rs`: create with the mode already set rather than
/// widening then narrowing, so the file is never briefly world-readable.
///
/// `.mode()` only applies when the file is *created*, so a leftover tmp from an
/// interrupted save would keep its old mode — hence the explicit
/// `set_permissions` afterwards.
fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        // Windows has no mode to set here; the file inherits the ACL of the
        // per-user profile directory it is created in.
        fs::write(path, bytes)
    }
}
include!("profiles_impl_01_part_01.rs");
include!("profiles_impl_01_part_02.rs");
include!("profiles_impl_01_part_03.rs");

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
    map.entry(key)
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
