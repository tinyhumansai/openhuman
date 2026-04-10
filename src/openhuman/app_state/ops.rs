use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use reqwest::{header::AUTHORIZATION, Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::api::config::effective_api_url;
use crate::api::jwt::{bearer_authorization_value, get_session_token};
use crate::openhuman::autocomplete::AutocompleteStatus;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::build_session_state;
use crate::openhuman::local_ai::LocalAiStatus;
use crate::openhuman::screen_intelligence::AccessibilityStatus;
use crate::openhuman::service::{ServiceState, ServiceStatus};
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[app_state]";
const APP_STATE_FILENAME: &str = "app-state.json";
static APP_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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
    pub primary_wallet_address: Option<String>,
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
    pub analytics_enabled: bool,
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
    pub primary_wallet_address: Option<Option<String>>,
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

fn load_stored_app_state(config: &Config) -> Result<StoredAppState, String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    load_stored_app_state_unlocked(config)
}

fn sync_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("failed to sync directory {}: {e}", parent.display()))?;
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
    Client::builder()
        .use_rustls_tls()
        .http1_only()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn resolve_base(config: &Config) -> Result<Url, String> {
    let base = effective_api_url(&config.api_url);
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

async fn build_runtime_snapshot(config: &Config) -> RuntimeSnapshot {
    let screen_intelligence = {
        let _ = crate::openhuman::screen_intelligence::global_engine()
            .apply_config(config.screen_intelligence.clone())
            .await;
        crate::openhuman::screen_intelligence::global_engine()
            .status()
            .await
    };

    let local_ai = match crate::openhuman::local_ai::rpc::local_ai_status(config).await {
        Ok(outcome) => outcome.value,
        Err(error) => {
            warn!("{LOG_PREFIX} local_ai status failed during snapshot: {error}");
            crate::openhuman::local_ai::LocalAiStatus::disabled(config)
        }
    };

    let autocomplete = crate::openhuman::autocomplete::global_engine()
        .status()
        .await;

    let service = match crate::openhuman::service::status(config) {
        Ok(status) => status,
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

    RuntimeSnapshot {
        screen_intelligence,
        local_ai,
        autocomplete,
        service,
    }
}

pub async fn snapshot() -> Result<RpcOutcome<AppStateSnapshot>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let auth = build_session_state(&config)?;
    let session_token = get_session_token(&config)?;
    let current_user = auth.user.clone().or(
        if let Some(token) = session_token.clone().filter(|t| !t.trim().is_empty()) {
            fetch_current_user(&config, &token).await?
        } else {
            None
        },
    );
    let local_state = load_stored_app_state(&config)?;
    let runtime = build_runtime_snapshot(&config).await;

    debug!(
        "{LOG_PREFIX} snapshot auth={} onboarding={} analytics={} wallet_present={} si_active={} local_ai_state={} autocomplete_phase={} service_state={:?}",
        auth.is_authenticated,
        config.onboarding_completed,
        config.observability.analytics_enabled,
        local_state.primary_wallet_address.is_some(),
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
            analytics_enabled: config.observability.analytics_enabled,
            local_state,
            runtime,
        },
        vec!["core app state snapshot fetched".to_string()],
    ))
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

    if let Some(primary_wallet_address) = patch.primary_wallet_address {
        current.primary_wallet_address = primary_wallet_address.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }

    if let Some(onboarding_tasks) = patch.onboarding_tasks {
        current.onboarding_tasks = onboarding_tasks;
    }

    save_stored_app_state_unlocked(&config, &current)?;

    debug!(
        "{LOG_PREFIX} local state updated encryption_key={} wallet={} onboarding_tasks={}",
        current.encryption_key.is_some(),
        current.primary_wallet_address.is_some(),
        current.onboarding_tasks.is_some()
    );

    Ok(RpcOutcome::new(
        current,
        vec!["core local app state updated".to_string()],
    ))
}
