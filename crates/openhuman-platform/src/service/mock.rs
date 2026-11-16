//! Deterministic, file-backed service manager used by E2E tests.
//! Enabled via `OPENHUMAN_SERVICE_MOCK`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{ServiceSpec, ServiceState, ServiceStatus};

const ENV_SERVICE_MOCK: &str = "OPENHUMAN_SERVICE_MOCK";
const ENV_SERVICE_MOCK_STATE_FILE: &str = "OPENHUMAN_SERVICE_MOCK_STATE_FILE";
const DEFAULT_STATE_FILE: &str = "service-mock-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct MockFailures {
    install: Option<String>,
    start: Option<String>,
    stop: Option<String>,
    status: Option<String>,
    uninstall: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct MockServiceState {
    installed: bool,
    running: bool,
    agent_running: bool,
    failures: MockFailures,
}

impl Default for MockServiceState {
    fn default() -> Self {
        Self {
            installed: false,
            running: false,
            agent_running: true,
            failures: MockFailures::default(),
        }
    }
}

pub(crate) fn is_enabled() -> bool {
    match std::env::var(ENV_SERVICE_MOCK) {
        Ok(raw) => matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

pub(crate) fn install(spec: &ServiceSpec) -> Result<ServiceStatus> {
    log::info!("[service-mock] install requested");
    let mut state = load_state(spec)?;
    maybe_fail(state.failures.install.as_deref(), "install")?;
    state.installed = true;
    state.running = false;
    save_state(spec, &state)?;
    log::info!("[service-mock] install completed: installed=true running=false");
    status(spec)
}

pub(crate) fn start(spec: &ServiceSpec) -> Result<ServiceStatus> {
    log::info!("[service-mock] start requested");
    let mut state = load_state(spec)?;
    maybe_fail(state.failures.start.as_deref(), "start")?;
    if !state.installed {
        log::warn!("[service-mock] start requested while not installed");
        return Ok(service_status_from_state(spec, &state));
    }
    state.running = true;
    save_state(spec, &state)?;
    log::info!("[service-mock] start completed: installed=true running=true");
    status(spec)
}

pub(crate) fn stop(spec: &ServiceSpec) -> Result<ServiceStatus> {
    log::info!("[service-mock] stop requested");
    let mut state = load_state(spec)?;
    maybe_fail(state.failures.stop.as_deref(), "stop")?;
    state.running = false;
    save_state(spec, &state)?;
    log::info!("[service-mock] stop completed: running=false");
    status(spec)
}

pub(crate) fn status(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let state = load_state(spec)?;
    maybe_fail(state.failures.status.as_deref(), "status")?;
    log::info!(
        "[service-mock] status requested: installed={} running={} agent_running={}",
        state.installed,
        state.running,
        state.agent_running
    );
    Ok(service_status_from_state(spec, &state))
}

pub(crate) fn uninstall(spec: &ServiceSpec) -> Result<ServiceStatus> {
    log::info!("[service-mock] uninstall requested");
    let mut state = load_state(spec)?;
    maybe_fail(state.failures.uninstall.as_deref(), "uninstall")?;
    state.installed = false;
    state.running = false;
    save_state(spec, &state)?;
    log::info!("[service-mock] uninstall completed: installed=false running=false");
    status(spec)
}

/// The mock's `agent_running` flag, or `None` when the mock is off.
///
/// Reads `OPENHUMAN_SERVICE_MOCK_STATE_FILE`, else `service-mock-state.json`
/// in the current directory: callers have no [`ServiceSpec`] to hand.
pub fn mock_agent_running() -> Option<bool> {
    if !is_enabled() {
        return None;
    }
    let path = state_file_path_without_spec();
    read_state_from_path(&path)
        .ok()
        .map(|state| state.agent_running)
}

fn maybe_fail(message: Option<&str>, operation: &str) -> Result<()> {
    if let Some(msg) = message {
        log::error!("[service-mock] forced failure for operation={operation}: {msg}");
        anyhow::bail!("[service-mock] {operation} failed: {msg}");
    }
    Ok(())
}

fn load_state(spec: &ServiceSpec) -> Result<MockServiceState> {
    let path = state_file_path(spec);
    if !path.exists() {
        let state = MockServiceState::default();
        save_state_to_path(&path, &state)?;
        log::info!(
            "[service-mock] created default state file at {}",
            path.display()
        );
        return Ok(state);
    }
    log::debug!("[service-mock] loading state from {}", path.display());
    read_state_from_path(&path)
}

fn save_state(spec: &ServiceSpec, state: &MockServiceState) -> Result<()> {
    save_state_to_path(&state_file_path(spec), state)
}

fn save_state_to_path(path: &Path, state: &MockServiceState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let bytes =
        serde_json::to_vec_pretty(state).context("failed serializing service mock state")?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed writing service mock state {}", path.display()))?;
    log::debug!(
        "[service-mock] wrote state to {} (installed={} running={} agent_running={})",
        path.display(),
        state.installed,
        state.running,
        state.agent_running
    );
    Ok(())
}

fn state_file_path(spec: &ServiceSpec) -> PathBuf {
    if let Some(path) = env_state_file() {
        return path;
    }

    spec.work_dir.join(DEFAULT_STATE_FILE)
}

fn state_file_path_without_spec() -> PathBuf {
    if let Some(path) = env_state_file() {
        return path;
    }
    PathBuf::from(DEFAULT_STATE_FILE)
}

fn env_state_file() -> Option<PathBuf> {
    let path = std::env::var(ENV_SERVICE_MOCK_STATE_FILE).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn read_state_from_path(path: &Path) -> Result<MockServiceState> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed reading service mock state {}", path.display()))?;
    let parsed = serde_json::from_str::<MockServiceState>(&raw).with_context(|| {
        format!(
            "failed parsing service mock state {} as JSON",
            path.display()
        )
    })?;
    Ok(parsed)
}

fn service_status_from_state(spec: &ServiceSpec, state: &MockServiceState) -> ServiceStatus {
    ServiceStatus {
        state: if !state.installed {
            ServiceState::NotInstalled
        } else if state.running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        },
        unit_path: mock_unit_path(spec),
        label: spec.label.clone(),
        details: Some("service mock backend".to_string()),
    }
}

/// The unit path the real manager for this OS would report.
#[cfg(target_os = "macos")]
fn mock_unit_path(spec: &ServiceSpec) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", spec.label)),
    )
}

#[cfg(target_os = "linux")]
fn mock_unit_path(spec: &ServiceSpec) -> Option<PathBuf> {
    Some(super::linux::user_unit_path(&spec.work_dir, &spec.label))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mock_unit_path(_spec: &ServiceSpec) -> Option<PathBuf> {
    None
}

#[cfg(test)]
#[path = "mock_tests.rs"]
mod mock_tests;
