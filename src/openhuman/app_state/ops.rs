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
use crate::openhuman::autocomplete::AutocompleteStatus;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::{
    is_local_session_token, load_app_session_profile, session_state_from_profile,
    session_token_from_profile,
};
use crate::openhuman::inference::LocalAiStatus;
use crate::openhuman::screen_intelligence::AccessibilityStatus;
use crate::openhuman::service::{ServiceState, ServiceStatus};
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[app_state]";
const APP_STATE_FILENAME: &str = "app-state.json";
const CURRENT_USER_REFRESH_TTL: Duration = Duration::from_secs(5);
const RUNTIME_SNAPSHOT_TTL: Duration = Duration::from_secs(2);
const AUTH_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const RUNTIME_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const SNAPSHOT_SUB_OP_TIMEOUT: Duration = Duration::from_secs(5);
static APP_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static CURRENT_USER_CACHE: Lazy<Mutex<Option<CachedCurrentUser>>> = Lazy::new(|| Mutex::new(None));
static RUNTIME_SNAPSHOT_CACHE: Lazy<Mutex<Option<CachedRuntimeSnapshot>>> =
    Lazy::new(|| Mutex::new(None));
static SNAPSHOT_REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct CachedRuntimeSnapshot {
    snapshot: RuntimeSnapshot,
    fetched_at: Instant,
}

#[derive(Debug, Clone)]
struct CachedCurrentUser {
    api_base: String,
    token: String,
    fetched_at: Instant,
    user: Value,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub auth: crate::openhuman::credentials::responses::AuthStateResponse,
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
    /// Mirror of `Config::meet.auto_orchestrator_handoff` — gates whether
    /// ending a Google Meet call hands the transcript to the orchestrator
    /// agent for proactive follow-up actions. Default `false`. See
    /// issue #1299.
    pub meet_auto_orchestrator_handoff: bool,
    pub local_state: StoredAppState,
    pub runtime: RuntimeSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub screen_intelligence: AccessibilityStatus,
    pub local_ai: LocalAiStatus,
    pub autocomplete: AutocompleteStatus,
    pub service: ServiceStatus,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredAppStatePatch {
    #[serde(default)]
    pub encryption_key: Option<Option<String>>,
    #[serde(default)]
    pub onboarding_tasks: Option<Option<StoredOnboardingTasks>>,
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

fn save_stored_app_state(config: &Config, state: &StoredAppState) -> Result<(), String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    save_stored_app_state_unlocked(config, state)
}

fn build_client() -> Result<Client, String> {
    // Platform-appropriate TLS backend — see [`crate::openhuman::tls`].
    crate::openhuman::tls::tls_client_builder()
        .http1_only()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
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

async fn fetch_current_user(config: &Config, token: &str) -> Result<Option<Value>, String> {
    let client = build_client()?;
    let base = resolve_base(config)?;
    let url = base
        .join("auth/me")
        .map_err(|e| format!("build URL failed: {e}"))?;
    let response = client
        .request(Method::GET, url.clone())
        .header(AUTHORIZATION, bearer_authorization_value(token))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("failed to read backend response body: {e}"))?;

    debug!("{LOG_PREFIX} GET /auth/me -> {}", status);

    if !status.is_success() {
        warn!(
            "{LOG_PREFIX} current user fetch failed: {} {}",
            status, text
        );
        return Ok(None);
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

async fn fetch_current_user_cached(config: &Config, token: &str) -> Result<Option<Value>, String> {
    let api_base = effective_backend_api_url(&config.api_url)
        .trim()
        .trim_end_matches('/')
        .to_string();

    {
        let cache = CURRENT_USER_CACHE.lock();
        if let Some(entry) = cache.as_ref() {
            if entry.api_base == api_base
                && entry.token == token
                && entry.fetched_at.elapsed() < CURRENT_USER_REFRESH_TTL
            {
                debug!(
                    "{LOG_PREFIX} using cached current user age_ms={}",
                    entry.fetched_at.elapsed().as_millis()
                );
                return Ok(Some(entry.user.clone()));
            }
        }
    }

    let fetched = sanitize_snapshot_user(fetch_current_user(config, token).await?);

    let mut cache = CURRENT_USER_CACHE.lock();
    match fetched.clone() {
        Some(user) => {
            debug!("{LOG_PREFIX} refreshed current user from backend");
            *cache = Some(CachedCurrentUser {
                api_base,
                token: token.to_string(),
                fetched_at: Instant::now(),
                user,
            });
        }
        None => {
            debug!("{LOG_PREFIX} backend returned empty current user; clearing cache");
            *cache = None;
        }
    }

    Ok(fetched)
}

/// Synchronous, network-free peek at the cached `auth_get_me` response,
/// returning only the identifying fields the prompt layer is allowed to
/// embed (`id`, `name`, `email`). Tokens stay locked behind the JWT
/// helpers — never returned through this path. See issue #926.
///
/// Returns `None` when no `auth_get_me` call has populated the cache
/// yet (CLI-only flows, fresh installs, signed-out sessions). The
/// cache TTL is **ignored** here intentionally — for prompt rendering
/// a slightly stale identity is fine; the freshness check only
/// matters for the snapshot RPC that fronts the React shell.
pub fn peek_cached_current_user_identity() -> Option<crate::openhuman::agent::prompts::UserIdentity>
{
    let cache = CURRENT_USER_CACHE.lock();
    let entry = cache.as_ref()?;
    let user = entry.user.as_object()?;

    let pluck = |key: &str| -> Option<String> {
        user.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let id = pluck("id")
        .or_else(|| pluck("user_id"))
        .or_else(|| pluck("userId"));
    let name = pluck("name")
        .or_else(|| pluck("displayName"))
        .or_else(|| pluck("display_name"))
        .or_else(|| pluck("full_name"))
        .or_else(|| pluck("fullName"));
    let email = pluck("email");

    let identity = crate::openhuman::agent::prompts::UserIdentity { id, name, email };
    if identity.is_empty() {
        None
    } else {
        Some(identity)
    }
}

async fn build_runtime_snapshot(config: &Config, req_id: u64) -> RuntimeSnapshot {
    {
        let cache = RUNTIME_SNAPSHOT_CACHE.lock();
        if let Some(entry) = cache.as_ref() {
            if entry.fetched_at.elapsed() < RUNTIME_SNAPSHOT_TTL {
                debug!(
                    "{LOG_PREFIX} build_runtime_snapshot: returning cached snapshot req_id={} age_ms={}",
                    req_id,
                    entry.fetched_at.elapsed().as_millis()
                );
                return entry.snapshot.clone();
            }
        }
    }

    let si_config = config.screen_intelligence.clone();
    let config_for_local_ai = config.clone();
    let config_for_autocomplete = config.clone();
    let config_for_service = config.clone();

    let t0 = Instant::now();

    let (screen_intelligence, local_ai, autocomplete, service) = tokio::join!(
        async {
            let t = Instant::now();
            let status = match tokio::time::timeout(SNAPSHOT_SUB_OP_TIMEOUT, async {
                let _ = crate::openhuman::screen_intelligence::global_engine()
                    .apply_config(si_config)
                    .await;
                crate::openhuman::screen_intelligence::global_engine()
                    .status()
                    .await
            })
            .await
            {
                Ok(s) => s,
                Err(_) => {
                    warn!(
                        "{LOG_PREFIX} screen_intelligence timed out after {}s; using degraded sub-snapshot req_id={}",
                        SNAPSHOT_SUB_OP_TIMEOUT.as_secs(),
                        req_id,
                    );
                    degraded_runtime_snapshot(config).screen_intelligence
                }
            };
            (status, t.elapsed().as_millis())
        },
        async {
            let t = Instant::now();
            let status = match tokio::time::timeout(
                SNAPSHOT_SUB_OP_TIMEOUT,
                crate::openhuman::inference::rpc::inference_status(&config_for_local_ai),
            )
            .await
            {
                Ok(Ok(outcome)) => outcome.value,
                Ok(Err(error)) => {
                    warn!("{LOG_PREFIX} local_ai status failed during snapshot: {error}");
                    crate::openhuman::inference::LocalAiStatus::disabled(&config_for_local_ai)
                }
                Err(_) => {
                    warn!(
                        "{LOG_PREFIX} local_ai timed out after {}s; using degraded sub-snapshot req_id={}",
                        SNAPSHOT_SUB_OP_TIMEOUT.as_secs(),
                        req_id,
                    );
                    crate::openhuman::inference::LocalAiStatus::disabled(&config_for_local_ai)
                }
            };
            (status, t.elapsed().as_millis())
        },
        async {
            let t = Instant::now();
            let status = match tokio::time::timeout(
                SNAPSHOT_SUB_OP_TIMEOUT,
                crate::openhuman::autocomplete::global_engine()
                    .status_with_config(&config_for_autocomplete),
            )
            .await
            {
                Ok(s) => s,
                Err(_) => {
                    warn!(
                        "{LOG_PREFIX} autocomplete timed out after {}s; using degraded sub-snapshot req_id={}",
                        SNAPSHOT_SUB_OP_TIMEOUT.as_secs(),
                        req_id,
                    );
                    degraded_runtime_snapshot(config).autocomplete
                }
            };
            (status, t.elapsed().as_millis())
        },
        async {
            let t = Instant::now();
            let status = tokio::task::spawn_blocking(move || {
                crate::openhuman::service::status(&config_for_service)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("service status task panicked")));
            let status = match status {
                Ok(s) => s,
                Err(error) => {
                    let message = error.to_string();
                    warn!("{LOG_PREFIX} service status failed during snapshot: {message}");
                    ServiceStatus {
                        state: ServiceState::Unknown(message.clone()),
                        unit_path: None,
                        label: "OpenHuman".to_string(),
                        details: Some(message),
                    }
                }
            };
            (status, t.elapsed().as_millis())
        }
    );

    let total_ms = t0.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} build_runtime_snapshot timings req_id={} si_ms={} local_ai_ms={} autocomplete_ms={} service_ms={} total_ms={}",
        req_id,
        screen_intelligence.1,
        local_ai.1,
        autocomplete.1,
        service.1,
        total_ms,
    );

    let snapshot = RuntimeSnapshot {
        screen_intelligence: screen_intelligence.0,
        local_ai: local_ai.0,
        autocomplete: autocomplete.0,
        service: service.0,
    };

    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: snapshot.clone(),
        fetched_at: Instant::now(),
    });

    snapshot
}

pub async fn snapshot() -> Result<RpcOutcome<AppStateSnapshot>, String> {
    let req_id = SNAPSHOT_REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let t_total = Instant::now();

    let t_config = Instant::now();
    let config = config_rpc::load_config_with_timeout().await?;
    let config_ms = t_config.elapsed().as_millis();

    let t_auth = Instant::now();
    // Load the `app-session` auth profile exactly once and derive both
    // the session-state view and the raw token from it. The previous
    // implementation called `build_session_state` + `get_session_token`
    // separately, which acquired the auth-profile file lock twice per
    // snapshot. On Windows this doubled the surface area for the
    // "Timed out waiting for auth profile lock" failure reported in
    // Sentry against `openhuman.app_state_snapshot`.
    //
    // `load_app_session_profile` calls `acquire_lock()`, which busy-waits
    // with `thread::sleep` for up to ~35s when the lock is contended. Calling
    // it directly on a tokio worker thread blocks that thread for the entire
    // wait, exhausting the thread pool under concurrent snapshot calls and
    // triggering `ERR_CONNECTION_TIMED_OUT` on all RPC connections.
    let config_for_profile = config.clone();
    let session_profile =
        tokio::task::spawn_blocking(move || load_app_session_profile(&config_for_profile))
            .await
            .unwrap_or_else(|e| Err(format!("[app_state] auth profile load task panicked: {e}")))?;
    let mut auth = session_state_from_profile(session_profile.as_ref());
    let session_token = session_token_from_profile(session_profile.as_ref());
    let stored_user = sanitize_snapshot_user(auth.user.clone());
    let auth_ms = t_auth.elapsed().as_millis();

    // Resolve the live current-user refresh and the runtime snapshot
    // CONCURRENTLY. Both touch the backend and both already fall back to local
    // data (stored_user / degraded runtime), so running them in parallel rather
    // than serially halves the worst-case bootstrap latency when the backend is
    // unreachable. Together with the fast auth-profile lock reclaim this keeps
    // the first `app_state_snapshot` from stranding the UI on "Initializing
    // OpenHuman" (the FE clears `isBootstrapping` on this call). `tokio::join!`
    // polls both on the current task — no extra threads.
    let t_enrich = Instant::now();
    let current_user_future = async {
        let Some(token) = session_token.clone().filter(|t| !t.trim().is_empty()) else {
            return stored_user.clone();
        };
        if is_local_session_token(&token) {
            return stored_user.clone();
        }
        match tokio::time::timeout(
            AUTH_FETCH_TIMEOUT,
            fetch_current_user_cached(&config, &token),
        )
        .await
        {
            Ok(Ok(fresh_user)) => fresh_user.or(stored_user.clone()),
            Ok(Err(error)) => {
                warn!("{LOG_PREFIX} current user refresh failed; using stored snapshot fallback: {error}");
                stored_user.clone()
            }
            Err(_) => {
                warn!(
                    "{LOG_PREFIX} current user fetch timed out after {}s; using stored snapshot fallback",
                    AUTH_FETCH_TIMEOUT.as_secs()
                );
                stored_user.clone()
            }
        }
    };
    let runtime_future = async {
        match tokio::time::timeout(
            RUNTIME_SNAPSHOT_TIMEOUT,
            build_runtime_snapshot(&config, req_id),
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(_) => {
                warn!(
                    "{LOG_PREFIX} build_runtime_snapshot timed out after {}s req_id={}; returning degraded runtime snapshot",
                    RUNTIME_SNAPSHOT_TIMEOUT.as_secs(),
                    req_id
                );
                degraded_runtime_snapshot(&config)
            }
        }
    };
    let (current_user, runtime) = tokio::join!(current_user_future, runtime_future);
    let enrich_ms = t_enrich.elapsed().as_millis();
    auth.user = current_user.clone();

    let t_local_state = Instant::now();
    let local_state = load_stored_app_state(&config)?;
    let local_state_ms = t_local_state.elapsed().as_millis();

    let total_ms = t_total.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} snapshot timings req_id={} config_ms={} auth_ms={} enrich_ms={} local_state_ms={} total_ms={}",
        req_id, config_ms, auth_ms, enrich_ms, local_state_ms, total_ms
    );

    debug!(
        "{LOG_PREFIX} snapshot req_id={} auth={} onboarding={} chat_onboarding={} analytics={} meet_handoff={} si_active={} local_ai_state={} autocomplete_phase={} service_state={:?}",
        req_id,
        auth.is_authenticated,
        config.onboarding_completed,
        config.chat_onboarding_completed,
        config.observability.analytics_enabled,
        config.meet.auto_orchestrator_handoff,
        runtime.screen_intelligence.session.active,
        runtime.local_ai.state,
        runtime.autocomplete.phase,
        runtime.service.state
    );

    Ok(RpcOutcome::new(
        AppStateSnapshot {
            auth,
            session_token,
            current_user,
            onboarding_completed: config.onboarding_completed,
            chat_onboarding_completed: config.chat_onboarding_completed,
            analytics_enabled: config.observability.analytics_enabled,
            meet_auto_orchestrator_handoff: config.meet.auto_orchestrator_handoff,
            local_state,
            runtime,
        },
        vec!["core app state snapshot fetched".to_string()],
    ))
}

fn degraded_runtime_snapshot(config: &Config) -> RuntimeSnapshot {
    use crate::openhuman::screen_intelligence::{
        AccessibilityFeatures, PermissionState, PermissionStatus, SessionStatus,
    };

    RuntimeSnapshot {
        screen_intelligence: AccessibilityStatus {
            platform_supported: cfg!(target_os = "macos"),
            permissions: PermissionStatus {
                screen_recording: PermissionState::Unknown,
                accessibility: PermissionState::Unknown,
                input_monitoring: PermissionState::Unknown,
                microphone: PermissionState::Unknown,
            },
            features: AccessibilityFeatures {
                screen_monitoring: false,
            },
            session: SessionStatus {
                active: false,
                started_at_ms: None,
                expires_at_ms: None,
                remaining_ms: None,
                ttl_secs: 0,
                panic_hotkey: config.screen_intelligence.panic_stop_hotkey.clone(),
                stop_reason: None,
                capture_count: 0,
                frames_in_memory: 0,
                last_capture_at_ms: None,
                last_context: None,
                last_window_title: None,
                vision_enabled: false,
                vision_state: "degraded".to_string(),
                vision_queue_depth: 0,
                last_vision_at_ms: None,
                last_vision_summary: None,
                vision_persist_count: 0,
                last_vision_persisted_key: None,
                last_vision_persist_error: None,
            },
            foreground_context: None,
            config: config.screen_intelligence.clone(),
            denylist: vec![],
            is_context_blocked: false,
            permission_check_process_path: None,
            core_process: None,
        },
        local_ai: crate::openhuman::inference::LocalAiStatus::disabled(config),
        autocomplete: crate::openhuman::autocomplete::AutocompleteStatus {
            platform_supported: cfg!(target_os = "macos"),
            enabled: config.autocomplete.enabled,
            running: false,
            phase: "degraded".to_string(),
            debounce_ms: config.autocomplete.debounce_ms,
            model_id: config.local_ai.chat_model_id.clone(),
            app_name: None,
            last_error: Some("snapshot timed out".to_string()),
            updated_at_ms: None,
            suggestion: None,
        },
        service: ServiceStatus {
            state: ServiceState::Unknown("snapshot timed out".to_string()),
            unit_path: None,
            label: "OpenHuman".to_string(),
            details: Some("runtime snapshot timed out".to_string()),
        },
    }
}

pub async fn update_local_state(
    patch: StoredAppStatePatch,
) -> Result<RpcOutcome<StoredAppState>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let _guard = APP_STATE_FILE_LOCK.lock();
    let mut current = load_stored_app_state_unlocked(&config)?;

    if let Some(encryption_key) = patch.encryption_key {
        current.encryption_key = encryption_key.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }

    if let Some(onboarding_tasks) = patch.onboarding_tasks {
        current.onboarding_tasks = onboarding_tasks;
    }

    save_stored_app_state_unlocked(&config, &current)?;

    debug!(
        "{LOG_PREFIX} local state updated encryption_key={} onboarding_tasks={}",
        current.encryption_key.is_some(),
        current.onboarding_tasks.is_some()
    );

    Ok(RpcOutcome::new(
        current,
        vec!["core local app state updated".to_string()],
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
