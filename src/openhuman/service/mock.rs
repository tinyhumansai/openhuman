//! Deterministic, file-backed service manager used by E2E tests.
//! Enabled via `OPENHUMAN_SERVICE_MOCK`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::openhuman::config::Config;

use super::common::SERVICE_LABEL;
use super::{ServiceState, ServiceStatus};

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

pub(crate) fn install(config: &Config) -> Result<ServiceStatus> {
    log::info!("[service-mock] install requested");
    let mut state = load_state(config)?;
    maybe_fail(state.failures.install.as_deref(), "install")?;
    state.installed = true;
    state.running = false;
    save_state(config, &state)?;
    log::info!("[service-mock] install completed: installed=true running=false");
    status(config)
}

pub(crate) fn start(config: &Config) -> Result<ServiceStatus> {
    log::info!("[service-mock] start requested");
    let mut state = load_state(config)?;
    maybe_fail(state.failures.start.as_deref(), "start")?;
    if !state.installed {
        log::warn!("[service-mock] start requested while not installed");
        return Ok(service_status_from_state(config, &state));
    }
    state.running = true;
    save_state(config, &state)?;
    log::info!("[service-mock] start completed: installed=true running=true");
    status(config)
}

pub(crate) fn stop(config: &Config) -> Result<ServiceStatus> {
    log::info!("[service-mock] stop requested");
    let mut state = load_state(config)?;
    maybe_fail(state.failures.stop.as_deref(), "stop")?;
    state.running = false;
    save_state(config, &state)?;
    log::info!("[service-mock] stop completed: running=false");
    status(config)
}

pub(crate) fn status(config: &Config) -> Result<ServiceStatus> {
    let state = load_state(config)?;
    maybe_fail(state.failures.status.as_deref(), "status")?;
    log::info!(
        "[service-mock] status requested: installed={} running={} agent_running={}",
        state.installed,
        state.running,
        state.agent_running
    );
    Ok(service_status_from_state(config, &state))
}

pub(crate) fn uninstall(config: &Config) -> Result<ServiceStatus> {
    log::info!("[service-mock] uninstall requested");
    let mut state = load_state(config)?;
    maybe_fail(state.failures.uninstall.as_deref(), "uninstall")?;
    state.installed = false;
    state.running = false;
    save_state(config, &state)?;
    log::info!("[service-mock] uninstall completed: installed=false running=false");
    status(config)
}

pub(crate) fn mock_agent_running() -> Option<bool> {
    if !is_enabled() {
        return None;
    }
    let path = state_file_path_without_config();
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

fn load_state(config: &Config) -> Result<MockServiceState> {
    let path = state_file_path(config);
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

fn save_state(config: &Config, state: &MockServiceState) -> Result<()> {
    save_state_to_path(&state_file_path(config), state)
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

fn state_file_path(config: &Config) -> PathBuf {
    if let Some(path) = env_state_file() {
        return path;
    }

    config
        .config_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DEFAULT_STATE_FILE)
}

fn state_file_path_without_config() -> PathBuf {
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

fn service_status_from_state(config: &Config, state: &MockServiceState) -> ServiceStatus {
    ServiceStatus {
        state: if !state.installed {
            ServiceState::NotInstalled
        } else if state.running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        },
        unit_path: mock_unit_path(config),
        label: mock_label().to_string(),
        details: Some("service mock backend".to_string()),
    }
}

#[cfg(target_os = "macos")]
fn mock_unit_path(_config: &Config) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{SERVICE_LABEL}.plist")),
    )
}

#[cfg(target_os = "linux")]
fn mock_unit_path(config: &Config) -> Option<PathBuf> {
    Some(
        config
            .config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("systemd")
            .join("user")
            .join("openhuman.service"),
    )
}

#[cfg(windows)]
fn mock_unit_path(_config: &Config) -> Option<PathBuf> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn mock_unit_path(_config: &Config) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn mock_label() -> &'static str {
    SERVICE_LABEL
}

#[cfg(target_os = "linux")]
fn mock_label() -> &'static str {
    "openhuman.service"
}

#[cfg(windows)]
fn mock_label() -> &'static str {
    "OpenHuman Core"
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn mock_label() -> &'static str {
    SERVICE_LABEL
}
