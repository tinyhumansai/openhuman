//! Local desktop enablement and permission-aware status.

use crate::core::runtime::DispatchCtx;
use std::fs;
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use serde::{Deserialize, Serialize};
use tinycomputer_bus::{names, ListAppsRequest, SnapshotRequest};

use crate::config::Config;

const DESKTOP_AGENT_TOOLS: &[&str] = &[
    "desktop_list_apps",
    "desktop_list_windows",
    "desktop_launch",
    "desktop_snapshot",
    "desktop_find",
    "desktop_goal",
    "desktop_continue_goal",
];

/// A registered desktop tool skips only approval parks when its explicit
/// desktop setting is off. Permission caps, denies, enabled state, OS grants,
/// and the action budget still run at their normal gates.
pub(crate) async fn approvals_disabled_for(ctx: &DispatchCtx, tool: &dyn tinytools::Tool) -> bool {
    if tool.family() != Some("desktop") || !DESKTOP_AGENT_TOOLS.contains(&tool.name()) {
        return false;
    }
    ctx.load_config()
        .await
        .is_ok_and(|config| approval_bypass_decision(&config, tool))
}

fn approval_bypass_decision(config: &Config, tool: &dyn tinytools::Tool) -> bool {
    tool.family() == Some("desktop")
        && DESKTOP_AGENT_TOOLS.contains(&tool.name())
        && !config.desktop.approvals_enabled
}

static STATE_LOCK: Mutex<()> = Mutex::new(());
static LOOPBACK_LISTENER: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static TEST_LOOPBACK_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that temporarily make the process-wide listener local.
#[cfg(test)]
pub(super) struct TestLoopbackGuard {
    previous: bool,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
pub(super) fn test_loopback_guard() -> TestLoopbackGuard {
    let lock = TEST_LOOPBACK_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    TestLoopbackGuard {
        previous: LOOPBACK_LISTENER.swap(true, Ordering::AcqRel),
        _lock: lock,
    }
}

#[cfg(test)]
impl Drop for TestLoopbackGuard {
    fn drop(&mut self) {
        set_listener_is_loopback(self.previous);
    }
}

/// The HTTP host reports its actual bound address before serving requests.
pub(crate) fn set_listener_is_loopback(value: bool) {
    LOOPBACK_LISTENER.store(value, Ordering::Release);
}

pub fn listener_is_loopback() -> bool {
    LOOPBACK_LISTENER.load(Ordering::Acquire)
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopStatus {
    pub supported: bool,
    pub enabled: bool,
    pub approvals_enabled: bool,
    pub platform: &'static str,
    pub module_state: String,
    pub accessibility: String,
    pub screen_recording: String,
    pub jev_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopProbe {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct LocalState {
    enabled: bool,
}

// Deliberately local, not on the storage port: this is the one machine's own
// consent to let the agent drive its desktop. A shared backend would let a
// flag set on one host enable control on another that never consented, and
// corrupt or missing state must keep failing closed. Allowlisted in
// `scripts/ci/check-storage-bypass.mjs`.
fn state_path(config: &Config) -> std::path::PathBuf {
    config
        .workspace_dir
        .join("state")
        .join("desktop-control.json")
}

/// Effective desktop control: the `[components] computer` ceiling AND this
/// machine's local consent. The flag can only remove access, never grant it.
/// Corrupt or unreadable local state fails closed, never enabling desktop access.
pub fn enabled(config: &Config) -> bool {
    if !config.components.computer {
        return false;
    }
    let _lock = STATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    fs::read_to_string(state_path(config))
        .ok()
        .and_then(|raw| serde_json::from_str::<LocalState>(&raw).ok())
        .is_some_and(|state| state.enabled)
}

fn save(config: &Config, value: bool) -> Result<(), String> {
    let _lock = STATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = state_path(config);
    let parent = path.parent().ok_or("desktop state path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("desktop state directory: {error}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("desktop state temporary file: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("desktop state permissions: {error}"))?;
    }
    serde_json::to_writer(&mut temporary, &LocalState { enabled: value })
        .map_err(|error| format!("desktop state serialization: {error}"))?;
    temporary
        .flush()
        .map_err(|error| format!("desktop state flush: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("desktop state sync: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("desktop state persist: {error}"))?;
    Ok(())
}

pub async fn set_enabled(config: &Config, value: bool) -> Result<DesktopStatus, String> {
    if value && !supported() {
        return Err(
            "desktop control requires macOS or Windows and a loopback core listener".to_owned(),
        );
    }
    save(config, value)?;
    tracing::info!(enabled = value, "[desktop] local enablement changed");
    Ok(status(config).await)
}

fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows")) && listener_is_loopback()
}

fn permission(data: &serde_json::Value, field: &str) -> String {
    let raw = data
        .get(field)
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    match raw {
        "granted" | "denied" | "not_required" | "unknown" => raw.to_owned(),
        _ => "unknown".to_owned(),
    }
}

/// On platforms without a per-app permission model for the given kind (e.g.
/// Windows has no macOS-style Accessibility or Screen Recording toggle),
/// treat an `unknown` report from the module as `not_required` so the UI
/// does not show a misleading "Not determined" state or send the user to
/// a generic settings page that has no relevant toggle.
pub(super) fn normalize_permission_for_platform(state: &str, platform: &str) -> String {
    if platform == "windows" && state == "unknown" {
        "not_required".to_owned()
    } else {
        state.to_owned()
    }
}

pub async fn status(config: &Config) -> DesktopStatus {
    status_with(config, supported(), || {
        crate::modules::desktop::permissions(config)
    })
    .await
}

async fn status_with<F, Fut>(
    config: &Config,
    platform_supported: bool,
    permissions: F,
) -> DesktopStatus
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<tinycomputer_bus::DesktopResponse, String>>,
{
    let local_enabled = enabled(config);
    let (module_state, mut reason) = crate::modules::desktop::state(config);
    let mut result = DesktopStatus {
        supported: platform_supported,
        enabled: local_enabled,
        approvals_enabled: config.desktop.approvals_enabled,
        platform: std::env::consts::OS,
        module_state,
        accessibility: "unknown".to_owned(),
        screen_recording: "unknown".to_owned(),
        jev_ready: crate::modules::desktop::jev_ready(config),
        reason: None,
    };
    if !result.supported {
        reason = Some(
            "desktop control requires macOS or Windows and a loopback core listener".to_owned(),
        );
    } else if local_enabled {
        match permissions().await {
            Ok(response) if response.ok => {
                if let Some(data) = response.data.as_ref() {
                    result.accessibility = normalize_permission_for_platform(
                        &permission(data, "accessibility"),
                        result.platform,
                    );
                    result.screen_recording = normalize_permission_for_platform(
                        &permission(data, "screen_recording"),
                        result.platform,
                    );
                }
                result.module_state = "ready".to_owned();
            }
            Ok(response) => reason = response.error.map(|error| error.message),
            Err(error) => reason = Some(error),
        }
        result.module_state = crate::modules::desktop::state(config).0;
    }
    result.reason = reason;
    result
}

pub async fn probe(config: &Config) -> DesktopProbe {
    probe_with(config, supported(), |member| async move {
        match member {
            names::methods::PERMISSIONS => crate::modules::desktop::permissions(config).await,
            names::methods::SNAPSHOT => {
                crate::modules::desktop::call(
                    config,
                    member,
                    SnapshotRequest {
                        skeleton: true,
                        ..SnapshotRequest::default()
                    },
                )
                .await
            }
            names::methods::LIST_APPS => {
                crate::modules::desktop::call(config, member, ListAppsRequest::default()).await
            }
            _ => unreachable!("probe calls only its three fixed module members"),
        }
    })
    .await
}

async fn probe_with<F, Fut>(config: &Config, platform_supported: bool, mut call: F) -> DesktopProbe
where
    F: FnMut(&'static str) -> Fut,
    Fut: std::future::Future<Output = Result<tinycomputer_bus::DesktopResponse, String>>,
{
    if !platform_supported || !enabled(config) {
        return DesktopProbe {
            ok: false,
            app_count: None,
            reason: Some("desktop control is unavailable or disabled".to_owned()),
        };
    }
    let permissions = match call(names::methods::PERMISSIONS).await {
        Ok(reply) if reply.ok => reply,
        Ok(reply) => {
            return DesktopProbe {
                ok: false,
                app_count: None,
                reason: reply.error.map(|error| error.message),
            }
        }
        Err(error) => {
            return DesktopProbe {
                ok: false,
                app_count: None,
                reason: Some(error),
            }
        }
    };
    let accessibility_state = permissions
        .data
        .as_ref()
        .map(|data| {
            normalize_permission_for_platform(
                &permission(data, "accessibility"),
                std::env::consts::OS,
            )
        })
        .unwrap_or_else(|| "unknown".to_owned());
    if accessibility_state != "granted" && accessibility_state != "not_required" {
        return DesktopProbe {
            ok: false,
            app_count: None,
            reason: Some("Accessibility permission is not granted to the core process".to_owned()),
        };
    }
    let snapshot = call(names::methods::SNAPSHOT).await;
    match snapshot {
        Ok(response) if response.ok => match call(names::methods::LIST_APPS).await {
            Ok(reply) if reply.ok => {
                let count = reply.data.as_ref().and_then(|data| {
                    data.get("apps")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                });
                DesktopProbe {
                    ok: count.is_some(),
                    app_count: count,
                    reason: count
                        .is_none()
                        .then(|| "desktop app list is missing".to_owned()),
                }
            }
            Ok(reply) => DesktopProbe {
                ok: false,
                app_count: None,
                reason: reply.error.map(|error| error.message),
            },
            Err(error) => DesktopProbe {
                ok: false,
                app_count: None,
                reason: Some(error),
            },
        },
        Ok(response) => DesktopProbe {
            ok: false,
            app_count: None,
            reason: response.error.map(|error| error.message),
        },
        Err(error) => DesktopProbe {
            ok: false,
            app_count: None,
            reason: Some(error),
        },
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
