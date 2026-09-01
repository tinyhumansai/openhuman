use std::collections::{BTreeMap, HashMap};
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use reqwest::{header::AUTHORIZATION, Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::bearer_authorization_value;
use crate::api::rest::user_id_from_profile_payload;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::inference::LocalAiStatus;
use crate::openhuman::platform::service::{ServiceState, ServiceStatus};
use crate::openhuman::security::credentials::session_support::{
    is_local_session_token, load_app_session_profile, session_state_from_profile,
    session_token_from_profile,
};
use crate::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[app_state]";
const APP_STATE_FILENAME: &str = "app-state.json";
const CURRENT_USER_REFRESH_TTL: Duration = Duration::from_secs(5);
// Runtime-status widgets (local AI / autocomplete / service) tolerate ~10s of
// staleness. A short TTL (was 2s < the ~2.4s build
// time) meant the cache was stale before it was even written, so the frontend's
// ~4s `app_state_snapshot` poll never hit the fast path and every poll re-ran
// the full 4-way fan-out (issue #4249 profiling: this, combined with the lack
// of a single-flight gate, pegged ~2 cores and starved the shared tokio runtime
// the agent harness runs on — the agent's turns stalled 50-100s between model
// calls even though inference itself was idle).
const RUNTIME_SNAPSHOT_TTL: Duration = Duration::from_secs(10);
const AUTH_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// First backoff step after the backend fails to answer `auth_get_me`.
///
/// Deliberately larger than both [`AUTH_FETCH_TIMEOUT`] and the frontend's
/// ~5s `app_state_snapshot` poll. That relationship is the whole point of the
/// backoff: with a shorter step, the next poll would find the window already
/// expired and pay the full timeout again, which is exactly the treadmill this
/// exists to stop (#5624 — 51 timeouts in one session, ~5s each).
const CURRENT_USER_BACKOFF_BASE: Duration = Duration::from_secs(10);
/// Ceiling on that backoff. Modest on purpose: this window is time during which
/// a recovered backend still will not be noticed, so it trades a bounded amount
/// of staleness for not stalling every poll. At the cap a 5s poll loop attempts
/// roughly one live fetch per twelve polls instead of one per poll.
const CURRENT_USER_BACKOFF_MAX: Duration = Duration::from_secs(60);
const RUNTIME_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const SNAPSHOT_SUB_OP_TIMEOUT: Duration = Duration::from_secs(5);
const PENDING_BACKEND_VALIDATION_FIELD: &str = "pendingBackendValidation";
const AUTH_ME_REVALIDATION_TRANSIENT_STATUSES: &[u16] = &[408, 429, 500, 502, 503, 504, 520];
static APP_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static CURRENT_USER_CACHE: Lazy<Mutex<Option<CachedCurrentUser>>> = Lazy::new(|| Mutex::new(None));
/// Negative counterpart to [`CURRENT_USER_CACHE`]: the last *availability*
/// failure against `auth_get_me`, so a client whose backend is unreachable stops
/// re-paying [`AUTH_FETCH_TIMEOUT`] on every snapshot poll.
///
/// Kept separate from the positive cache rather than folded into it because the
/// two have different lifetimes and different readers —
/// [`peek_cached_current_user_identity`] must keep serving the last known
/// identity throughout an outage, and it reads only the positive cache.
static CURRENT_USER_FAILURE: Lazy<Mutex<Option<CurrentUserFailure>>> =
    Lazy::new(|| Mutex::new(None));
static RUNTIME_SNAPSHOT_CACHE: Lazy<Mutex<Option<CachedRuntimeSnapshot>>> =
    Lazy::new(|| Mutex::new(None));
/// Single-flight gate for the runtime-snapshot rebuild. Concurrent callers whose
/// cache read missed serialize here so only ONE runs the expensive sub-op
/// fan-out; the rest wait, then re-read the cache the winner populated (see the
/// double-check in `build_runtime_snapshot`). This is an async mutex because the
/// guard is held across `.await` points (the sub-op `join`). Without it, every
/// overlapping `app_state_snapshot` poll launched its own build — the rebuild
/// stampede described on `RUNTIME_SNAPSHOT_TTL`.
static RUNTIME_SNAPSHOT_REBUILD: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));
static SNAPSHOT_REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct CachedRuntimeSnapshot {
    snapshot: RuntimeSnapshot,
    fetched_at: Instant,
    /// Config identity (`workspace_dir`) the snapshot was built for. The cache
    /// holds one entry process-wide, so a snapshot built for one config must
    /// never be served to another — otherwise a different user/workspace (or an
    /// E2E test with an injected service mock) reads a stale, foreign runtime.
    config_key: PathBuf,
}

#[derive(Debug, Clone)]
struct CachedCurrentUser {
    api_base: String,
    token: String,
    fetched_at: Instant,
    user: Value,
}

#[derive(Debug, Clone)]
enum SnapshotCurrentUser {
    User(Option<Value>),
    DeferredSessionRejected,
}

impl SnapshotCurrentUser {
    fn user(user: Option<Value>) -> Self {
        Self::User(user)
    }
}

type SnapshotCurrentUserResult = (SnapshotCurrentUser, Option<Box<Config>>);

fn snapshot_current_user_result(user: Option<Value>) -> SnapshotCurrentUserResult {
    (SnapshotCurrentUser::user(user), None)
}

#[derive(Debug, Clone)]
enum CurrentUserFetchError {
    Rejected(String),
    TransientResponse(String),
    FetchFailed(String),
}

impl CurrentUserFetchError {
    fn message(&self) -> &str {
        match self {
            CurrentUserFetchError::Rejected(message)
            | CurrentUserFetchError::TransientResponse(message)
            | CurrentUserFetchError::FetchFailed(message) => message,
        }
    }
}

impl CurrentUserFetchError {
    /// Whether this failure says the *backend was not reachable or not healthy*,
    /// as opposed to saying something about our credentials.
    ///
    /// Only these are worth backing off. A [`Rejected`](Self::Rejected) is the
    /// backend answering, in time, that the token is no good — it drives the
    /// deferred-session cleanup at the snapshot caller, and replaying it from a
    /// cache would either delay that cleanup or, worse, hand the caller a
    /// different variant than the one the backend actually produced.
    fn is_availability_failure(&self) -> bool {
        match self {
            CurrentUserFetchError::TransientResponse(_) | CurrentUserFetchError::FetchFailed(_) => {
                true
            }
            CurrentUserFetchError::Rejected(_) => false,
        }
    }
}

/// The last availability failure against `auth_get_me`, keyed the same way the
/// positive cache is so that changing environment or signing in as someone else
/// bypasses it rather than inheriting someone else's outage.
#[derive(Debug, Clone)]
struct CurrentUserFailure {
    api_base: String,
    token: String,
    failed_at: Instant,
    /// Failures in an unbroken run, counted from 1. Drives the backoff width.
    consecutive: u32,
    /// Replayed verbatim while the window is open, so a caller that matches on
    /// the variant sees what the backend really produced.
    error: CurrentUserFetchError,
}

/// How long a run of `consecutive` failures suppresses the next live attempt.
///
/// Doubles from [`CURRENT_USER_BACKOFF_BASE`] and saturates at
/// [`CURRENT_USER_BACKOFF_MAX`]. `consecutive` is 1-based; 0 is treated as 1 so
/// the function has no surprising zero-length window.
fn current_user_backoff(consecutive: u32) -> Duration {
    let steps = consecutive.saturating_sub(1).min(16);
    CURRENT_USER_BACKOFF_BASE
        .saturating_mul(2u32.saturating_pow(steps))
        .min(CURRENT_USER_BACKOFF_MAX)
}

/// The recorded failure for `(api_base, token)` if its backoff window is still
/// open, in which case the caller should return it instead of going to the
/// network.
fn suppressed_current_user_failure(
    api_base: &str,
    token: &str,
) -> Option<(CurrentUserFetchError, u32, Duration)> {
    let failure = CURRENT_USER_FAILURE.lock();
    let entry = failure.as_ref()?;
    if entry.api_base != api_base || entry.token != token {
        return None;
    }
    let window = current_user_backoff(entry.consecutive);
    let elapsed = entry.failed_at.elapsed();
    (elapsed < window).then(|| (entry.error.clone(), entry.consecutive, window - elapsed))
}

/// Record an availability failure, extending the run when it is the same
/// `(api_base, token)` and starting a new one when it is not.
///
/// A [`CurrentUserFetchError::Rejected`] is ignored — see
/// [`CurrentUserFetchError::is_availability_failure`].
fn record_current_user_failure(api_base: &str, token: &str, error: CurrentUserFetchError) {
    if !error.is_availability_failure() {
        return;
    }
    let mut failure = CURRENT_USER_FAILURE.lock();
    let consecutive = match failure.as_ref() {
        Some(entry) if entry.api_base == api_base && entry.token == token => {
            entry.consecutive.saturating_add(1)
        }
        _ => 1,
    };
    *failure = Some(CurrentUserFailure {
        api_base: api_base.to_string(),
        token: token.to_string(),
        failed_at: Instant::now(),
        consecutive,
        error,
    });
}

/// Forget any recorded failure, so the next poll goes straight to the network.
///
/// Called on every success and on sign-out. Missing either one is the failure
/// mode that matters here: a stale record outliving its cause keeps the app on
/// the stored snapshot after the backend has already come back.
fn clear_current_user_failure() {
    *CURRENT_USER_FAILURE.lock() = None;
}

/// Record the timeout path's failure.
///
/// The timeout is applied by the snapshot caller, wrapping the whole of
/// `fetch_current_user_cached`, so when it fires that future is **dropped
/// mid-flight** and nothing inside it runs — including the failure recording on
/// its error path. Without this call the backoff would never engage for the one
/// case #5624 is actually about, which is timeouts rather than returned errors.
fn note_current_user_timeout(config: &Config, token: &str) {
    record_current_user_failure(
        &current_user_api_base(config),
        token,
        CurrentUserFetchError::FetchFailed(format!(
            "request timed out after {}s",
            AUTH_FETCH_TIMEOUT.as_secs()
        )),
    );
}

/// The cache key both the positive and negative current-user caches are keyed
/// on. Factored out so the snapshot caller, which records the timeout path, and
/// the fetch itself cannot drift apart on how the base URL is normalised.
fn current_user_api_base(config: &Config) -> String {
    effective_backend_api_url(&config.api_url)
        .trim()
        .trim_end_matches('/')
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredOnboardingTasks {
    #[serde(default)]
    pub accessibility_permission_granted: bool,
    #[serde(default)]
    pub local_model_consent_given: bool,
    #[serde(default)]
    pub local_model_download_started: bool,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub connected_sources: Vec<String>,
    #[serde(default)]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredAppState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_tasks: Option<StoredOnboardingTasks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyring_consent: Option<crate::openhuman::security::keyring_consent::ConsentPreference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub auth: crate::openhuman::security::credentials::responses::AuthStateResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_user: Option<Value>,
    pub onboarding_completed: bool,
    /// Deprecated — the welcome agent has been removed. Retained in the
    /// snapshot for backward compatibility with frontend code that still
    /// reads it. This value may be `false` in newer configs; routing no
    /// longer depends on this field.
    pub chat_onboarding_completed: bool,
    pub analytics_enabled: bool,
    pub local_state: StoredAppState,
    pub keyring_status: crate::openhuman::security::keyring_consent::KeyringStatus,
    pub runtime: RuntimeSnapshot,
    /// Process + component health, folded into this snapshot so the frontend
    /// hydrates the daemon-health store from the same poll instead of running a
    /// second `health_snapshot` poller. Fields stay snake_case (the type has no
    /// camelCase rename) to match the frontend's existing health parser.
    pub health: crate::openhuman::platform::health::HealthSnapshot,
    /// `true` when this session's config loader had to recover a corrupted
    /// `config.toml` (renamed to `.corrupted.<ts>` and reset to defaults / a
    /// backup). Latched at boot so it stays reported even after the file is
    /// healed; the frontend raises a one-shot "settings were reset" notice
    /// (#5167). Serialized as `configRecovered`.
    pub config_recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub local_ai: LocalAiStatus,
    pub service: ServiceStatus,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredAppStatePatch {
    #[serde(default)]
    pub encryption_key: Option<Option<String>>,
    #[serde(default)]
    pub onboarding_tasks: Option<Option<StoredOnboardingTasks>>,
    #[serde(default)]
    pub keyring_consent:
        Option<Option<crate::openhuman::security::keyring_consent::ConsentPreference>>,
}

fn app_state_path(config: &Config) -> Result<PathBuf, String> {
    let state_dir = config.workspace_dir.join("state");
    fs::create_dir_all(&state_dir).map_err(|e| {
        format!(
            "failed to create workspace state dir {}: {e}",
            state_dir.display()
        )
    })?;
    Ok(state_dir.join(APP_STATE_FILENAME))
}

fn corrupted_app_state_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    path.with_extension(format!("json.corrupted.{timestamp}"))
}

fn quarantine_corrupted_app_state(path: &Path, reason: &str) {
    let quarantine_path = corrupted_app_state_path(path);
    warn!(
        "{LOG_PREFIX} quarantining corrupted app state {} -> {} ({reason})",
        path.display(),
        quarantine_path.display()
    );

    if let Err(rename_error) = fs::rename(path, &quarantine_path) {
        warn!(
            "{LOG_PREFIX} failed to quarantine {} via rename: {}",
            path.display(),
            rename_error
        );
        if let Err(remove_error) = fs::remove_file(path) {
            warn!(
                "{LOG_PREFIX} failed to remove unreadable app state {}: {}",
                path.display(),
                remove_error
            );
        }
    }
}

fn load_stored_app_state_unlocked(config: &Config) -> Result<StoredAppState, String> {
    let path = app_state_path(config)?;
    if !path.exists() {
        return Ok(StoredAppState::default());
    }

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to read {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_app_state(&path, &error.to_string());
            return Ok(StoredAppState::default());
        }
    };

    match serde_json::from_str::<StoredAppState>(&raw) {
        Ok(state) => Ok(state),
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to parse {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_app_state(&path, &error.to_string());
            Ok(StoredAppState::default())
        }
    }
}

pub(crate) fn load_stored_app_state(config: &Config) -> Result<StoredAppState, String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    load_stored_app_state_unlocked(config)
}

fn sync_parent_dir(path: &Path) -> Result<(), String> {
    // Directory fsync is a POSIX-only durability guarantee — on Unix we
    // open the parent dir and call `sync_all()` so the rename of the
    // temp file into place is persisted even if the host crashes before
    // the next buffer flush. On Windows, opening a directory as a
    // regular file requires `FILE_FLAG_BACKUP_SEMANTICS` which
    // `std::fs::File::open` does not set, so the call fails with
    // "Access is denied. (os error 5)". Since Windows uses a different
    // durability model (and `NamedTempFile::persist` issues an atomic
    // MoveFileEx which is already durable enough for our config files),
    // we skip the fsync entirely on non-Unix and return Ok. Mirrors the
    // existing `sync_directory` guard in `config/schema/load.rs`.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("failed to sync directory {}: {e}", parent.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn save_stored_app_state_unlocked(config: &Config, state: &StoredAppState) -> Result<(), String> {
    let path = app_state_path(config)?;
    let payload = serde_json::to_string_pretty(state)
        .map_err(|e| format!("failed to serialize app state: {e}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("failed to resolve parent dir for {}", path.display()))?;
    let mut temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create temp file in {}: {e}", parent.display()))?;
    temp_file
        .write_all(payload.as_bytes())
        .map_err(|e| format!("failed to write temp app state for {}: {e}", path.display()))?;
    temp_file
        .as_file_mut()
        .sync_all()
        .map_err(|e| format!("failed to sync temp app state for {}: {e}", path.display()))?;
    sync_parent_dir(&path)?;
    temp_file.persist(&path).map_err(|e| {
        format!(
            "failed to persist app state {}: {}",
            path.display(),
            e.error
        )
    })?;
    sync_parent_dir(&path)?;
    Ok(())
}

pub fn save_app_state(config: &Config, state: &StoredAppState) -> Result<(), String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    save_stored_app_state_unlocked(config, state)
}

fn build_client() -> Result<Client, String> {
    // Platform-appropriate TLS backend — see [`crate::openhuman::util::tls`].
    crate::openhuman::util::tls::tls_client_builder()
        .http1_only()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        // `GET /auth/me` is backend traffic like any other, so it carries the
        // product identity. This client is hand-rolled rather than obtained
        // from `BackendOAuthClient`, so it inherits nothing from that path's
        // default headers — see [`crate::api::product`]. Set here rather than
        // at the one call site because every user of this builder is
        // backend-bound by construction (`resolve_base` resolves the backend
        // API URL and nothing else).
        .default_headers(crate::api::product::product_identity_headers())
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn resolve_base(config: &Config) -> Result<Url, String> {
    let base = effective_backend_api_url(&config.api_url);
    let mut parsed =
        Url::parse(base.trim()).map_err(|e| format!("invalid api_url '{}': {e}", base))?;
    if !parsed.path().ends_with('/') && parsed.path() != "/" {
        let normalized = format!("{}/", parsed.path());
        parsed.set_path(&normalized);
    }
    Ok(parsed)
}

async fn fetch_current_user(
    config: &Config,
    token: &str,
) -> Result<Option<Value>, CurrentUserFetchError> {
    let client = build_client().map_err(CurrentUserFetchError::FetchFailed)?;
    let base = resolve_base(config).map_err(CurrentUserFetchError::FetchFailed)?;
    let url = base
        .join("auth/me")
        .map_err(|e| CurrentUserFetchError::FetchFailed(format!("build URL failed: {e}")))?;
    let response = client
        .request(Method::GET, url.clone())
        .header(AUTHORIZATION, bearer_authorization_value(token))
        .send()
        .await
        .map_err(|e| CurrentUserFetchError::FetchFailed(format!("request failed: {e}")))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| {
        CurrentUserFetchError::FetchFailed(format!("failed to read backend response body: {e}"))
    })?;

    debug!("{LOG_PREFIX} GET /auth/me -> {}", status);

    if !status.is_success() {
        let message = format!("{status} {text}");
        warn!(
            "{LOG_PREFIX} current user fetch failed: {} {}",
            status, text
        );
        return if AUTH_ME_REVALIDATION_TRANSIENT_STATUSES.contains(&status.as_u16()) {
            Err(CurrentUserFetchError::TransientResponse(message))
        } else {
            Err(CurrentUserFetchError::Rejected(message))
        };
    }

    let raw: Value =
        serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.to_string()));
    let user = raw
        .as_object()
        .and_then(|obj| obj.get("data"))
        .cloned()
        .unwrap_or(raw);
    Ok(Some(user))
}

fn sanitize_snapshot_user(user: Option<Value>) -> Option<Value> {
    match user {
        Some(Value::Object(map)) if map.is_empty() => None,
        Some(Value::Null) => None,
        other => other,
    }
}

fn snapshot_user_pending_backend_validation(user: Option<&Value>) -> bool {
    user.and_then(Value::as_object)
        .and_then(|obj| obj.get(PENDING_BACKEND_VALIDATION_FIELD))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn clear_pending_backend_validation_flag(mut user: Value) -> Value {
    if let Value::Object(ref mut map) = user {
        map.remove(PENDING_BACKEND_VALIDATION_FIELD);
    }
    user
}

fn pending_session_user_id_for_cleanup(
    stored_user: Option<&Value>,
    metadata: &BTreeMap<String, String>,
) -> Option<String> {
    stored_user
        .and_then(user_id_from_profile_payload)
        .or_else(|| {
            metadata
                .get("user_id")
                .map(String::as_str)
                .map(str::trim)
                .filter(|user_id| !user_id.is_empty())
                .map(str::to_string)
        })
}

fn config_state_dir(config: &Config) -> Option<PathBuf> {
    config.config_path.parent().map(Path::to_path_buf)
}

fn same_config_state_dir(a: &Config, b: &Config) -> bool {
    config_state_dir(a) == config_state_dir(b)
}

fn config_dir_for_workspace_env() -> Option<PathBuf> {
    let workspace = std::env::var_os("OPENHUMAN_WORKSPACE")?;
    if workspace.as_os_str().is_empty() {
        return None;
    }

    let workspace_dir = PathBuf::from(workspace);
    let workspace_config_dir = workspace_dir.clone();
    if workspace_config_dir.join("config.toml").exists() {
        return Some(workspace_config_dir);
    }

    if let Some(parent) = workspace_dir.parent() {
        let legacy_dir = parent.join(".openhuman");
        if legacy_dir.join("config.toml").exists()
            || workspace_dir
                .file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return Some(legacy_dir);
        }
    }

    Some(workspace_config_dir)
}

fn config_is_workspace_env_scoped(config: &Config) -> bool {
    let Some(config_dir) = config_state_dir(config) else {
        return false;
    };
    config_dir_for_workspace_env()
        .as_deref()
        .is_some_and(|env_config_dir| env_config_dir == config_dir)
}

async fn activate_revalidated_user_dir(user_id: &str) -> Result<Config, String> {
    let root_dir = crate::openhuman::config::default_root_openhuman_dir()
        .map_err(|error| format!("failed to locate default root: {error}"))?;
    let previous_active = crate::openhuman::config::read_active_user_id(&root_dir);
    let user_dir = crate::openhuman::config::user_openhuman_dir(&root_dir, user_id);
    fs::create_dir_all(&user_dir).map_err(|error| {
        format!("failed to create user directory for revalidated pending session user_id={user_id}: {error}")
    })?;
    crate::openhuman::config::write_active_user_id(&root_dir, user_id).map_err(|error| {
        format!("failed to write active_user.toml for revalidated pending session user_id={user_id}: {error}")
    })?;

    debug!(
        "{LOG_PREFIX} activated user directory for revalidated pending session user_id={user_id}"
    );
    if previous_active.is_none() {
        let pre_ws = crate::openhuman::config::pre_login_user_dir(&root_dir).join("workspace");
        if let Err(error) = crate::openhuman::memory::conversations::purge_threads(pre_ws) {
            debug!(
                "{LOG_PREFIX} pre-login conversation purge skipped after pending session revalidation: {error}"
            );
        }
    }

    Config::load_from_default_paths().await.map_err(|error| {
        format!("failed to reload config after pending session user activation: {error}")
    })
}
