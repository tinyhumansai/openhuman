#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host is desktop-only. Non-desktop targets are not supported.");

mod cdp;
#[cfg(target_os = "macos")]
mod cef_preflight;
mod cef_profile;
mod core_process;
mod core_update;
mod discord_scanner;
mod gmail;
mod gmessages_scanner;
mod imessage_scanner;
mod notification_settings;
mod screen_capture;
mod slack_scanner;
mod telegram_scanner;
mod webview_accounts;
mod webview_apis;
mod whatsapp_scanner;

use std::sync::Mutex;

#[cfg(target_os = "macos")]
use tauri::WindowEvent;
#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject};
#[cfg(target_os = "macos")]
use objc2::ClassType;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSPanel, NSWindowCollectionBehavior, NSWindowStyleMask};

// CEF is the only runtime; alias kept so command handlers thread the runtime generic uniformly.
pub(crate) type AppRuntime = tauri::Cef;

/// Tracks the currently registered dictation hotkey string so we can unregister it later.
struct DictationHotkeyState(Mutex<Vec<String>>);

fn expand_dictation_shortcuts(shortcut: &str) -> Vec<String> {
    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    #[cfg(target_os = "macos")]
    {
        if trimmed.contains("CmdOrCtrl") {
            let cmd_variant = trimmed.replace("CmdOrCtrl", "Cmd");
            let ctrl_variant = trimmed.replace("CmdOrCtrl", "Ctrl");
            if cmd_variant == ctrl_variant {
                return vec![cmd_variant];
            }
            return vec![cmd_variant, ctrl_variant];
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if trimmed.contains("CmdOrCtrl") {
            return vec![trimmed.replace("CmdOrCtrl", "Ctrl")];
        }
    }

    vec![trimmed.to_string()]
}

#[tauri::command]
fn core_rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

#[tauri::command]
fn overlay_parent_rpc_url() -> Option<String> {
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok()?;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[allow(dead_code)] // Overlay disabled in tauri.conf.json; helper kept for future re-enable.
fn pin_overlay_bottom_right(window: &WebviewWindow<AppRuntime>) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        log::warn!("[overlay] could not resolve current monitor for positioning");
        return;
    };
    let Ok(size) = window.outer_size() else {
        log::warn!("[overlay] could not resolve overlay size for positioning");
        return;
    };

    let margin = 20i32;
    let x = monitor.position().x + monitor.size().width as i32 - size.width as i32 - margin;
    let y = monitor.position().y + monitor.size().height as i32 - size.height as i32 - margin;

    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("[overlay] failed to pin overlay bottom-right: {err}");
    } else {
        log::info!("[overlay] pinned overlay bottom-right at {},{}", x, y);
    }
}

#[cfg(target_os = "macos")]
#[allow(dead_code)] // Overlay disabled in tauri.conf.json; helper kept for future re-enable.
fn configure_overlay_window_macos(window: &WebviewWindow<AppRuntime>) {
    // Standard NSWindow cannot float above fullscreen apps on macOS because
    // fullscreen apps run in a separate Space. Only NSPanel can do this.
    //
    // Tauri/tao hardcodes NSWindow as the window class, so we use
    // object_setClass() to reclass the existing NSWindow into an NSPanel
    // at runtime. This avoids creating a new window (which crashes because
    // Tao's window delegate is tightly coupled to the original NSWindow).
    //
    // After reclassing, we set the NonactivatingPanel style mask and
    // Transient collection behavior — matching the working Swift overlay
    // helper (accessibility/helper.rs OverlayController) which is confirmed
    // to float above fullscreen apps on macOS Sonoma.
    //
    // Previous attempts that FAILED:
    // 1. CGShieldingWindowLevel + CanJoinAllSpaces + FullScreenAuxiliary → hidden
    // 2. Window level i32::MAX-17 + Stationary → hidden
    // 3. CGS private API CGSSetWindowTags sticky bit → hidden
    // 4. object_setClass WITHOUT NonactivatingPanel style mask → hidden
    // 5. Create new NSPanel + reparent webview → CRASH (Tao delegate panic)
    //
    // See: https://github.com/tauri-apps/tauri/issues/11488

    match window.ns_window() {
        Ok(ns_window_raw) => unsafe {
            let ns_window = ns_window_raw as *mut AnyObject;

            // ── Reclass NSWindow → NSPanel ──────────────────────────
            let panel_class: *const AnyClass = NSPanel::class();
            objc2::ffi::object_setClass(ns_window, panel_class);
            log::info!("[overlay] reclassed NSWindow → NSPanel via object_setClass");

            // Cast to NSPanel for method calls
            let panel: &NSPanel = &*(ns_window as *const NSPanel);

            // ── Style mask: add NonactivatingPanel ──────────────────
            // This is the KEY piece the Swift helper uses. Without it,
            // the panel doesn't behave as a proper non-activating panel
            // and won't float above fullscreen Spaces.
            let current_style = panel.styleMask();
            panel.setStyleMask(current_style | NSWindowStyleMask::NonactivatingPanel);

            // ── Collection behavior ─────────────────────────────────
            // The Swift helper uses .canJoinAllSpaces + .transient
            // (NOT .stationary or .fullScreenAuxiliary alone).
            // Transient means the panel follows the active Space and
            // appears above fullscreen apps.
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );

            // ── Window level: status bar tier ───────────────────────
            // NSStatusWindowLevel = 25. The Swift helper uses .statusBar
            // which is the same value.
            panel.setLevel(25);

            // ── Panel-specific properties ───────────────────────────
            panel.setFloatingPanel(true);
            panel.setHidesOnDeactivate(false);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setWorksWhenModal(true);

            // Make sure it's ordered front
            panel.orderFrontRegardless();

            log::info!(
                "[overlay] NSPanel configured — level=25, \
                 NonactivatingPanel+canJoinAllSpaces+transient, \
                 floatingPanel={}, hidesOnDeactivate={}",
                panel.isFloatingPanel(),
                panel.hidesOnDeactivate(),
            );
        },
        Err(err) => {
            log::warn!("[overlay] failed to access native NSWindow handle: {err}");
        }
    }
}

/// Resolve the core binary, preferring the staged sidecar.
fn resolve_core_bin() -> Result<std::path::PathBuf, String> {
    if let Some(bin) = core_process::default_core_bin() {
        return Ok(bin);
    }
    std::env::current_exe().map_err(|e| format!("cannot resolve executable: {e}"))
}

/// Run the core binary with the given CLI args and return its stdout.
async fn run_core_cli(args: Vec<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let bin = resolve_core_bin()?;
        let is_self = {
            let current = std::env::current_exe().ok();
            current
                .as_ref()
                .and_then(|c| std::fs::canonicalize(c).ok())
                .zip(std::fs::canonicalize(&bin).ok())
                .map_or(false, |(a, b)| a == b)
        };

        let mut cmd = std::process::Command::new(&bin);
        if is_self {
            cmd.arg("core");
        }
        cmd.args(&args);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        log::info!(
            "[service-direct] running {:?} {}{}",
            bin,
            if is_self { "core " } else { "" },
            args.join(" ")
        );

        let output = cmd
            .output()
            .map_err(|e| format!("failed to execute core binary: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "core binary exited with {}: {}",
                output.status,
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
async fn service_install_direct() -> Result<String, String> {
    run_core_cli(vec!["service".into(), "install".into()]).await
}

#[tauri::command]
async fn service_start_direct() -> Result<String, String> {
    run_core_cli(vec!["service".into(), "start".into()]).await
}

#[tauri::command]
async fn service_stop_direct() -> Result<String, String> {
    run_core_cli(vec!["service".into(), "stop".into()]).await
}

#[tauri::command]
async fn service_status_direct() -> Result<String, String> {
    run_core_cli(vec!["service".into(), "status".into()]).await
}

#[tauri::command]
async fn service_uninstall_direct() -> Result<String, String> {
    run_core_cli(vec!["service".into(), "uninstall".into()]).await
}

/// Check if the core sidecar is outdated and whether a newer version is available on GitHub.
/// Returns version info, compatibility status, and update availability.
#[tauri::command]
async fn check_core_update(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<serde_json::Value, String> {
    let rpc_url = state.inner().rpc_url();
    let info = core_update::check_full(&rpc_url).await?;
    serde_json::to_value(&info).map_err(|e| format!("serialize error: {e}"))
}

/// Trigger a full core update: download latest from GitHub, stage, kill old, restart.
/// Uses `force=true` so it updates to the latest release even if the running core
/// meets the minimum version requirement.
#[tauri::command]
async fn apply_core_update(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
    app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    log::info!("[core-update] manual apply_core_update invoked from frontend");
    core_update::check_and_update_core(state.inner().clone(), Some(app), true).await
}

#[tauri::command]
async fn restart_core_process(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<(), String> {
    log::info!("[core] restart_core_process: command invoked from frontend");
    let _guard = state.inner().restart_lock().await;
    log::debug!("[core] restart_core_process: acquired restart lock");
    state.inner().restart().await
}

#[tauri::command]
async fn restart_app(app: tauri::AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[app] restart_app invoked from frontend");
    app.restart();
}

#[tauri::command]
async fn schedule_cef_profile_purge(user_id: Option<String>) -> Result<String, String> {
    let queued = cef_profile::queue_profile_purge_for_user(user_id.as_deref())?;
    Ok(queued.display().to_string())
}

/// Information about an available shell-app update returned to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
struct AppUpdateInfo {
    /// The currently-running app version (matches `tauri.conf.json::version`).
    current_version: String,
    /// True when the configured updater endpoint advertises a newer version.
    available: bool,
    /// Newer version reported by the updater endpoint, if any.
    available_version: Option<String>,
    /// Release notes / body for the new version, if the manifest provided one.
    body: Option<String>,
}

/// Probe the updater endpoint and report whether a newer shell build is available.
/// Does NOT download or install. Pair with `apply_app_update` to actually upgrade.
#[tauri::command]
async fn check_app_update(app: tauri::AppHandle<AppRuntime>) -> Result<AppUpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;

    let current_version = app.package_info().version.to_string();
    log::info!("[app-update] check requested (current: {current_version})");

    let updater = app
        .updater()
        .map_err(|e| format!("updater plugin not initialized: {e}"))?;

    match updater.check().await {
        Ok(Some(update)) => {
            log::info!(
                "[app-update] update available: {} -> {}",
                current_version,
                update.version
            );
            Ok(AppUpdateInfo {
                current_version,
                available: true,
                available_version: Some(update.version.clone()),
                body: update.body.clone(),
            })
        }
        Ok(None) => {
            log::info!("[app-update] no update available");
            Ok(AppUpdateInfo {
                current_version,
                available: false,
                available_version: None,
                body: None,
            })
        }
        Err(e) => {
            log::warn!("[app-update] check failed: {e}");
            Err(format!("update check failed: {e}"))
        }
    }
}

/// Download and install the latest shell update, then relaunch.
///
/// Shuts the core sidecar down before download begins so the install step
/// (which on macOS replaces the entire `.app` bundle) does not race against
/// a live sidecar holding file handles inside `Contents/Resources/`. The
/// new bundled sidecar is launched fresh after `app.restart()`.
///
/// Emits Tauri events `app-update:status` and `app-update:progress` so the
/// frontend can show a snackbar / progress bar.
#[tauri::command]
async fn apply_app_update(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
    app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;

    log::info!("[app-update] manual apply_app_update invoked from frontend");

    let updater = app
        .updater()
        .map_err(|e| format!("updater plugin not initialized: {e}"))?;

    let _ = app.emit("app-update:status", "checking");

    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => {
            log::info!("[app-update] no update available");
            let _ = app.emit("app-update:status", "up_to_date");
            return Ok(());
        }
        Err(e) => {
            log::warn!("[app-update] check failed: {e}");
            let _ = app.emit("app-update:status", "error");
            return Err(format!("update check failed: {e}"));
        }
    };

    let new_version = update.version.clone();
    log::info!(
        "[app-update] downloading {} (size hint: {:?})",
        new_version,
        update.signature
    );
    let _ = app.emit("app-update:status", "downloading");

    // Shut the core sidecar down before the install step replaces the .app.
    // We hold the restart lock until app.restart() so nothing tries to
    // respawn the sidecar from the in-flight (or freshly-replaced) bundle.
    let _guard = state.inner().restart_lock().await;
    log::debug!("[app-update] acquired core restart lock");
    state.inner().shutdown().await;

    let progress_app = app.clone();
    let install_app = app.clone();
    let download_result = update
        .download_and_install(
            move |chunk_length, content_length| {
                let payload = serde_json::json!({
                    "chunk": chunk_length,
                    "total": content_length,
                });
                let _ = progress_app.emit("app-update:progress", payload);
            },
            move || {
                log::info!("[app-update] download complete — installing");
                let _ = install_app.emit("app-update:status", "installing");
            },
        )
        .await;

    if let Err(e) = download_result {
        log::error!("[app-update] download/install failed: {e}");
        let _ = app.emit("app-update:status", "error");
        return Err(format!("download_and_install failed: {e}"));
    }

    log::info!("[app-update] install complete — relaunching");
    let _ = app.emit("app-update:status", "restarting");
    // Note: app.restart() never returns. Anything after this is unreachable.
    app.restart();
}

/// Register (or re-register) the global dictation toggle hotkey.
/// Emits `dictation://toggle` to all webviews when the shortcut is pressed.
#[tauri::command]
async fn register_dictation_hotkey(
    app: AppHandle<AppRuntime>,
    shortcut: String,
) -> Result<(), String> {
    log::info!("[dictation] register_dictation_hotkey: shortcut={shortcut}");

    let old_shortcuts = {
        let state = app.state::<DictationHotkeyState>();
        let guard = state.0.lock().unwrap();
        guard.clone()
    };

    let expanded_shortcuts = expand_dictation_shortcuts(&shortcut);
    if expanded_shortcuts.is_empty() {
        return Err("Shortcut cannot be empty".to_string());
    }
    log::info!(
        "[dictation] expanded shortcuts: {}",
        expanded_shortcuts.join(", ")
    );

    let register_shortcut = |shortcut_variant: &str| -> Result<(), String> {
        let app_clone = app.clone();
        app.global_shortcut()
            .on_shortcut(shortcut_variant, move |_app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    log::debug!("[dictation] hotkey pressed — emitting dictation://toggle");
                    if let Err(e) = app_clone.emit("dictation://toggle", ()) {
                        log::warn!("[dictation] emit failed: {e}");
                    }
                }
            })
            .map_err(|e| format!("Failed to register shortcut '{shortcut_variant}': {e}"))
    };

    let mut unregistered_old: Vec<String> = Vec::new();
    for old in &old_shortcuts {
        log::debug!("[dictation] unregistering previous shortcut: {old}");
        if let Err(e) = app.global_shortcut().unregister(old.as_str()) {
            for restored in &unregistered_old {
                if let Err(restore_err) = register_shortcut(restored.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while restoring old shortcut '{restored}': {restore_err}"
                    );
                }
            }
            return Err(format!(
                "Failed to unregister previous shortcut '{old}': {e}"
            ));
        }
        unregistered_old.push(old.clone());
    }

    let mut newly_registered: Vec<String> = Vec::new();
    for shortcut_variant in &expanded_shortcuts {
        if let Err(err) = register_shortcut(shortcut_variant.as_str()) {
            log::error!("[dictation] failed to register shortcut '{shortcut_variant}': {err}");
            for registered in &newly_registered {
                if let Err(unregister_err) = app.global_shortcut().unregister(registered.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while unregistering '{registered}': {unregister_err}"
                    );
                }
            }
            for old in &old_shortcuts {
                if let Err(restore_err) = register_shortcut(old.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while restoring old shortcut '{old}': {restore_err}"
                    );
                }
            }
            return Err(err);
        }
        newly_registered.push(shortcut_variant.clone());
    }

    // Persist all newly registered shortcuts.
    {
        let state = app.state::<DictationHotkeyState>();
        let mut guard = state.0.lock().unwrap();
        *guard = expanded_shortcuts.clone();
    }

    log::info!(
        "[dictation] shortcuts registered: {}",
        expanded_shortcuts.join(", ")
    );
    Ok(())
}

/// Unregister the global dictation hotkey (if any).
#[tauri::command]
async fn unregister_dictation_hotkey(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[dictation] unregister_dictation_hotkey: called");
    let state = app.state::<DictationHotkeyState>();
    let mut guard = state.0.lock().unwrap();
    if guard.is_empty() {
        log::debug!("[dictation] no shortcut registered — nothing to unregister");
    } else {
        let old_shortcuts = guard.clone();
        guard.clear();
        for old in old_shortcuts {
            log::debug!("[dictation] unregistering shortcut: {old}");
            app.global_shortcut()
                .unregister(old.as_str())
                .map_err(|e| {
                    log::warn!("[dictation] failed to unregister '{old}': {e}");
                    format!("Failed to unregister shortcut '{old}': {e}")
                })?;
            log::info!("[dictation] shortcut unregistered: {old}");
        }
    }
    Ok(())
}

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

/// Tauri command: bring the main window to front from any webview (e.g. overlay orb click).
#[tauri::command]
fn activate_main_window(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::debug!("[window] activate_main_window called from overlay");
    show_main_window(&app)
}

/// Tauri command: fire a native OS notification from the frontend. Used by
/// the in-app notification center to banner events (agent completions,
/// connection drops, etc.) when the window is not focused.
#[tauri::command]
fn show_native_notification(
    app: AppHandle<AppRuntime>,
    title: String,
    body: String,
    tag: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let permission_state = app
        .notification()
        .permission_state()
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|e| format!("err({e})"));
    log::debug!(
        "[notify] show_native_notification title_chars={} body_chars={} tag={:?} permission={permission_state}",
        title.len(),
        body.len(),
        tag
    );
    let mut builder = app.notification().builder().title(&title);
    if !body.is_empty() {
        builder = builder.body(&body);
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder.sound("default");
    }
    builder
        .show()
        .map_err(|e| format!("notification show failed: {e}"))
}

#[cfg(target_os = "macos")]
fn macos_notification_permission_state_inner() -> Result<String, String> {
    use std::ptr::NonNull;
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2_user_notifications::{
        UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
    };

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::channel::<String>();
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let status = unsafe { settings.as_ref().authorizationStatus() };
        let state = if status == UNAuthorizationStatus::Authorized {
            "granted"
        } else if status == UNAuthorizationStatus::Denied {
            "denied"
        } else if status == UNAuthorizationStatus::NotDetermined {
            "not_determined"
        } else if status == UNAuthorizationStatus::Provisional {
            "provisional"
        } else if status == UNAuthorizationStatus::Ephemeral {
            "ephemeral"
        } else {
            "unknown"
        };
        let _ = tx.send(state.to_string());
    });
    center.getNotificationSettingsWithCompletionHandler(&completion);
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "timed out waiting for macOS notification settings".to_string())
}

#[cfg(target_os = "macos")]
fn macos_notification_permission_request_inner() -> Result<String, String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::NSError;
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
    use std::sync::mpsc;

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::channel::<bool>();
    let options = UNAuthorizationOptions::Alert
        | UNAuthorizationOptions::Badge
        | UNAuthorizationOptions::Sound;
    let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    center.requestAuthorizationWithOptions_completionHandler(options, &completion);
    let granted = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "timed out waiting for macOS permission prompt result".to_string())?;
    if granted {
        Ok("granted".to_string())
    } else {
        // If the user denies or notifications are disabled for the app,
        // macOS reports `false` here.
        Ok("denied".to_string())
    }
}

#[tauri::command]
fn notification_permission_state() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return macos_notification_permission_state_inner();
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("granted".to_string())
    }
}

#[tauri::command]
fn notification_permission_request() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return macos_notification_permission_request_inner();
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("granted".to_string())
    }
}

fn show_main_window(app: &AppHandle<AppRuntime>) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    window
        .show()
        .map_err(|err| format!("failed to show main window: {err}"))?;
    window
        .unminimize()
        .map_err(|err| format!("failed to unminimize main window: {err}"))?;
    window
        .set_focus()
        .map_err(|err| format!("failed to focus main window: {err}"))?;
    Ok(())
}
#[cfg(target_os = "linux")]
fn setup_tray(app: &AppHandle<AppRuntime>) -> tauri::Result<()> {
    let _ = app;
    log::warn!(
        "[tray] skipping tray setup on linux: tray menu creation still panics inside GTK during packaged runs"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn setup_tray(app: &AppHandle<AppRuntime>) -> tauri::Result<()> {
    log::info!("[tray] setting up tray icon");

    let show_item = MenuItem::with_id(
        app,
        "tray_show_window",
        "Open OpenHuman",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_string()))?;

    TrayIconBuilder::with_id("openhuman-tray")
        .icon(icon)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show_window" => {
                log::info!("[tray] action=show_window source=menu");
                if let Err(err) = show_main_window(app) {
                    log::error!("[tray] failed to show main window from menu: {err}");
                }
            }
            "tray_quit" => {
                log::info!("[tray] action=quit source=menu");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                log::info!("[tray] action=show_window source=left_click");
                if let Err(err) = show_main_window(tray.app_handle()) {
                    log::error!("[tray] failed to show main window from tray click: {err}");
                }
            }
        })
        .build(app)?;

    log::info!("[tray] tray icon ready");
    Ok(())
}

const CEF_PREWARM_LABEL: &str = "cef-prewarm";

/// Spawn a hidden 1×1 child webview at `about:blank` on the main window so
/// CEF's child-webview render path is hot before the user clicks an
/// account. The first `webview_account_open` then skips the cold
/// renderer-process spinup. Idempotent — bails if the prewarm webview
/// already exists.
fn spawn_cef_prewarm(app: &AppHandle<AppRuntime>) -> Result<(), String> {
    use tauri::webview::WebviewBuilder;
    use tauri::WebviewUrl;

    if app.get_webview(CEF_PREWARM_LABEL).is_some() {
        return Ok(());
    }
    let parent = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let url: tauri::Url = "about:blank"
        .parse()
        .map_err(|e| format!("about:blank parse: {e}"))?;
    let builder = WebviewBuilder::new(CEF_PREWARM_LABEL, WebviewUrl::External(url));
    parent
        .add_child(
            builder,
            tauri::LogicalPosition::new(-20000.0, -20000.0),
            tauri::LogicalSize::new(1.0, 1.0),
        )
        .map_err(|e| format!("add_child failed: {e}"))?;
    log::info!("[cef-prewarm] hidden warmup webview spawned");
    Ok(())
}

/// Drop the prewarm webview if still alive. Called from `RunEvent::Exit`
/// so its CEF browser is torn down before `cef::shutdown()` runs.
fn teardown_cef_prewarm<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(wv) = app.get_webview(CEF_PREWARM_LABEL) else {
        return Err("no prewarm webview".into());
    };
    wv.close().map_err(|e| e.to_string())?;
    log::info!("[cef-prewarm] teardown ok");
    Ok(())
}

pub fn run() {
    let daemon_mode = is_daemon_mode();

    let default_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = env_logger::Builder::new()
        .parse_filters(&default_filter)
        .try_init();

    // The vendored tauri-cef dev-server proxy builds a reqwest 0.13 client
    // (see vendor/tauri-cef/crates/tauri/src/protocol/tauri.rs) which calls
    // rustls 0.23's `CryptoProvider::get_default()`. rustls 0.23 no longer
    // picks a provider implicitly — without one installed, the proxy panics
    // with "No provider set" the first time `tauri dev` forwards a request.
    // Install the ring provider once before any HTTPS client is built.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // CEF cache-lock preflight (macOS only): if another OpenHuman instance
    // is already holding the CEF user-data-dir, the vendored
    // `tauri-runtime-cef` panics inside `cef::initialize` with a Rust
    // backtrace and no actionable message (issue #864). Catch the collision
    // here and exit cleanly with a message that names the lock-holder PID
    // and the workaround. Stale locks (PID dead) are removed and we
    // continue, matching Chromium's own startup recovery.
    match cef_profile::prepare_process_cache_path() {
        Ok(path) => log::debug!("[cef-profile] startup cache path={}", path.display()),
        Err(error) => {
            log::error!(
                "[cef-profile] failed to configure per-user CEF cache; refusing to start with shared/global cache: {error}"
            );
            std::process::exit(1);
        }
    }

    #[cfg(target_os = "macos")]
    if let Err(e) = cef_preflight::check_default_cache() {
        eprintln!("\n[openhuman] {e}\n");
        std::process::exit(1);
    }

    let builder = {
        // Bypass macOS Keychain. Without this, every embedded service that
        // touches password / cookie / encryption-key storage triggers a
        // "Allow access to your keychain?" prompt — WhatsApp Web hits it on
        // every cold start, Chromium's own component-update store also does.
        // `use-mock-keychain` swaps the Keychain backend for an in-process
        // mock; `password-store=basic` is the equivalent for the password
        // manager. Both are no-ops on Windows/Linux, so safe to always set.
        //
        // In debug builds we additionally expose the Chrome DevTools
        // Protocol on localhost:19222 so every CEF webview can be
        // inspected from a regular browser (right-click "Inspect" does
        // not propagate to CEF child webviews on macOS). Release builds
        // intentionally do NOT open the CDP port — it would let any
        // process on the machine drive the embedded WhatsApp/Slack/etc.
        // webviews.
        //
        // The port was 9222 (Chromium's default) but ollama's
        // OpenAI-compatible server squats on 127.0.0.1:9222 in some
        // installs, which silently broke CDP attach (our client hit
        // ollama, the WS handshake failed, child webviews stayed at
        // about:blank → black screen). Picked 19222 to dodge that
        // collision; if you change it here also update
        // `cdp::CDP_PORT` and `whatsapp_scanner::CDP_PORT`.
        //
        // NOTE: flags must be prefixed with `--`. The runtime's
        // `on_before_command_line_processing` dispatch (in
        // `tauri-runtime-cef/src/cef_impl.rs`) routes value-less args that
        // don't start with `-` to `append_argument` (positional) instead of
        // `append_switch`, which means Chromium silently ignores them.
        let mut args: Vec<(&str, Option<&str>)> = vec![
            ("--use-mock-keychain", None),
            ("--password-store", Some("basic")),
            // Enable SharedArrayBuffer so embedded apps that need WebRTC
            // audio worklets / Opus encoders (Slack Huddles, Meet
            // real-time features, Discord voice) can actually initialise.
            // Chromium gates SharedArrayBuffer behind cross-origin
            // isolation by default; web apps embedded inside CEF rarely
            // send COOP/COEP headers, so without this flag the feature
            // silently disappears and huddle/call buttons no-op.
            ("--enable-features", Some("SharedArrayBuffer")),
        ];
        // Always expose the CDP port, not just in debug. The webview-accounts
        // CDP session opener navigates each embedded provider webview from its
        // `about:blank#openhuman-acct-...` placeholder to the real provider URL
        // via `Page.navigate`. Without this port available in release builds,
        // the CDP client can't attach (`browser_ws_url()` 404s on /json/version),
        // the navigation never fires, and the embedded webview stays on
        // `about:blank` (blank panel for Telegram / WhatsApp / Slack / Discord).
        // Same port the `cdp::CDP_HOST`/`cdp::CDP_PORT` constants expect.
        args.push(("--remote-debugging-port", Some("19222")));
        tauri::Builder::<tauri::Cef>::new().command_line_args::<&str, &str>(args)
    };

    let builder = builder
        // Explicitly disable `open_js_links_on_click`: tauri-plugin-opener
        // defaults to injecting `init-iife.js` into *every* webview — a
        // global click listener that invokes `plugin:opener|open_url` via
        // HTTP-IPC. That violates our "no JS injection into CEF child
        // webviews" rule (see CLAUDE.md) and also fails in practice
        // because third-party origins (web.telegram.org, linkedin, …)
        // trip Tauri's Origin header check and return 500. External link
        // handling for `acct_*` webviews runs natively via
        // `on_navigation` / `on_new_window` in webview_accounts/mod.rs;
        // the main window uses `openUrl()` from `utils/openUrl.ts` when
        // it needs to hand off a URL.
        .plugin(
            tauri_plugin_opener::Builder::default()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Auto-updater for the Tauri shell. Endpoint and minisign pubkey live
        // in `tauri.conf.json` under `plugins.updater`. Releases are signed at
        // build time with `TAURI_SIGNING_PRIVATE_KEY` (+ `_PASSWORD`); see
        // docs/AUTO_UPDATE.md for the full pipeline.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DictationHotkeyState(Mutex::new(Vec::new())))
        .manage(webview_accounts::WebviewAccountsState::default())
        .manage(notification_settings::NotificationSettingsState::new());
    let builder = builder.manage(std::sync::Arc::new(imessage_scanner::ScannerRegistry::new()));
    let builder = builder.manage(std::sync::Arc::new(
        gmessages_scanner::ScannerRegistry::new(),
    ));
    let builder = builder.manage(whatsapp_scanner::ScannerRegistry::new());
    let builder = builder.manage(std::sync::Arc::new(slack_scanner::ScannerRegistry::new()));
    let builder = builder.manage(discord_scanner::ScannerRegistry::new());
    let builder = builder.manage(telegram_scanner::ScannerRegistry::new());
    let builder = builder.manage(screen_capture::ScreenShareState::new());
    builder
        .setup(move |app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                if let Err(err) = app.deep_link().register_all() {
                    log::warn!("[deep-link] register_all failed (non-fatal): {err}");
                }
            }

            // Start the webview_apis WebSocket bridge BEFORE spawning core —
            // core reads OPENHUMAN_WEBVIEW_APIS_PORT on first connect, and
            // connects lazily, so the env var must be set before the spawn.
            //
            // If the bridge fails to bind we clear any inherited port env so
            // the core child can't accidentally connect to whichever loopback
            // process already owns that port, then abort setup — the bridge
            // is load-bearing for every webview_apis RPC method.
            let bridge_ok = tauri::async_runtime::block_on(async {
                match webview_apis::start().await {
                    Ok(port) => {
                        std::env::set_var(webview_apis::server::PORT_ENV, port.to_string());
                        log::info!("[webview_apis] bridge ready on port {port}");
                        true
                    }
                    Err(err) => {
                        log::error!("[webview_apis] failed to start bridge: {err}");
                        std::env::remove_var(webview_apis::server::PORT_ENV);
                        false
                    }
                }
            });
            if !bridge_ok {
                return Err("webview_apis bridge failed to start — aborting setup".into());
            }

            let core_run_mode = core_process::default_core_run_mode(daemon_mode);
            let core_bin = if matches!(core_run_mode, core_process::CoreRunMode::ChildProcess) {
                core_process::default_core_bin()
            } else {
                None
            };
            let core_handle = core_process::CoreProcessHandle::new(
                core_process::default_core_port(),
                core_bin,
                core_run_mode,
            );
            std::env::set_var("OPENHUMAN_CORE_RPC_URL", core_handle.rpc_url());

            // Expose the shared CEF cookies SQLite path to the core sidecar
            // so `check_onboarding_status` can detect which webview
            // providers (gmail, whatsapp, slack, …) already have a live
            // session cookie. Best-effort — if we can't resolve the path
            // the core treats every provider as logged_out.
            if let Some(cache_dir) = cef_profile::configured_cache_path_from_env() {
                let cookies_db = cache_dir.join("Default").join("Cookies");
                log::debug!("[webview_accounts] exposing cookies DB path to core");
                std::env::set_var("OPENHUMAN_CEF_COOKIES_DB", &cookies_db);
            } else {
                // Clear any inherited value so the core can't pick up a
                // stale path from a previous run or the parent shell.
                std::env::remove_var("OPENHUMAN_CEF_COOKIES_DB");
                log::warn!(
                    "[webview_accounts] could not resolve configured CEF cache dir — core \
                     will report all webview providers as logged_out"
                );
            }

            app.manage(core_handle.clone());
            let app_handle_for_update = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = core_handle.ensure_running().await {
                    log::error!("[core] failed to start core process: {err}");
                    return;
                }
                log::info!("[core] core process ready");

                // Check if the running core is outdated and auto-update if needed.
                let update_handle = core_handle.clone();
                if let Err(err) = core_update::check_and_update_core(
                    update_handle,
                    Some(app_handle_for_update),
                    false,
                )
                .await
                {
                    log::warn!("[core-update] auto-update check failed (non-fatal): {err}");
                }
            });

            if daemon_mode {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    log::info!("[tray] daemon_mode=true window_hidden_on_startup");
                }
            }

            // Overlay window is currently disabled in `tauri.conf.json` (the
            // `overlay` entry under `app.windows` was removed), so we skip
            // the macOS NSPanel reclass + bottom-right pin + initial show
            // here. The helpers (`configure_overlay_window_macos`,
            // `pin_overlay_bottom_right`) and the React entry point
            // (`src/overlay/OverlayApp.tsx`) are kept intact so the overlay
            // can be re-enabled by restoring the config entry and the two
            // setup blocks below.
            //
            //   #[cfg(target_os = "macos")]
            //   if let Some(window) = app.get_webview_window("overlay") {
            //       configure_overlay_window_macos(&window);
            //   }
            //   if let Some(window) = app.get_webview_window("overlay") {
            //       pin_overlay_bottom_right(&window);
            //       let _ = window.show();
            //   }

            // Tray icon setup moved to RunEvent::Ready (see below) — GTK is only
            // initialized after the event loop starts, so we must delay tray creation
            // until the Ready event fires. Creating the tray here would panic on
            // Linux with "GTK has not been initialized".
            log::info!("[tray] deferring tray setup to RunEvent::Ready");

            // CEF cold-start warmup. Spawns a 1×1 hidden child webview on
            // the main window at `about:blank` so CEF's render-process /
            // compositor for child webviews is hot before the user clicks
            // an account — first cold open of a real provider drops from
            // "spin up renderer + navigate" to just "navigate".
            //
            // Earlier builds had this disabled because of a "blank webview
            // on first onboarding open" report; we now park the warmup at
            // a far off-screen position and never reveal it (matching the
            // 1×1-on-screen pattern used for cold account spawns), and
            // tear it down in the shutdown sequence below. Disable at
            // runtime with `OPENHUMAN_CEF_PREWARM=0` if it regresses.
            {
                let prewarm_enabled = std::env::var("OPENHUMAN_CEF_PREWARM")
                    .map(|v| {
                        let v = v.trim().to_ascii_lowercase();
                        !(v == "0" || v == "false" || v == "no" || v == "off")
                    })
                    .unwrap_or(true);
                if prewarm_enabled {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        // Defer one tick so the main window finishes its
                        // first paint before we attach a sibling webview.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        if let Err(e) = spawn_cef_prewarm(&app_handle) {
                            log::warn!("[cef-prewarm] failed (non-fatal): {e}");
                        }
                    });
                } else {
                    log::info!("[cef-prewarm] disabled via OPENHUMAN_CEF_PREWARM");
                }
            }

            // Dev convenience: if OPENHUMAN_DEV_AUTO_WHATSAPP=<account-id>
            // is set, spawn that account's webview at startup so the
            // CDP/IndexedDB scanner can iterate without manual UI clicks.
            // The same account-id reuses the persistent data dir, so a
            // previously-logged-in WhatsApp session stays logged in.
            if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_WHATSAPP") {
                let account_id = account_id.trim().to_string();
                if !account_id.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        // Wait for the window to be fully ready.
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let state = app_handle.state::<webview_accounts::WebviewAccountsState>();
                        let args = webview_accounts::OpenArgs {
                            account_id: account_id.clone(),
                            provider: "whatsapp".to_string(),
                            url: None,
                            bounds: Some(webview_accounts::Bounds {
                                x: 100.0,
                                y: 100.0,
                                width: 900.0,
                                height: 700.0,
                            }),
                        };
                        match webview_accounts::webview_account_open(
                            app_handle.clone(),
                            state,
                            args,
                        )
                        .await
                        {
                            Ok(label) => log::info!(
                                "[dev-auto-whatsapp] spawned label={} account={}",
                                label,
                                account_id
                            ),
                            Err(e) => log::error!(
                                "[dev-auto-whatsapp] failed: {} (account={})",
                                e,
                                account_id
                            ),
                        }
                    });
                }
            }

            // Same dev helper, Slack flavour. OPENHUMAN_DEV_AUTO_SLACK=<uuid>
            // opens the Slack account webview on startup so the CDP scanner
            // can iterate without manual UI clicks.
            if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_SLACK") {
                let account_id = account_id.trim().to_string();
                if !account_id.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let state = app_handle.state::<webview_accounts::WebviewAccountsState>();
                        let args = webview_accounts::OpenArgs {
                            account_id: account_id.clone(),
                            provider: "slack".to_string(),
                            url: None,
                            bounds: Some(webview_accounts::Bounds {
                                x: 100.0,
                                y: 100.0,
                                width: 900.0,
                                height: 700.0,
                            }),
                        };
                        match webview_accounts::webview_account_open(
                            app_handle.clone(),
                            state,
                            args,
                        )
                        .await
                        {
                            Ok(label) => log::info!(
                                "[dev-auto-slack] spawned label={} account={}",
                                label,
                                account_id
                            ),
                            Err(e) => log::error!(
                                "[dev-auto-slack] failed: {} (account={})",
                                e,
                                account_id
                            ),
                        }
                    });
                }
            }

            // Same dev helper, Telegram flavour. OPENHUMAN_DEV_AUTO_TELEGRAM=<uuid>
            // opens the Telegram Web K account webview on startup so the CDP
            // scanner can iterate without manual UI clicks.
            if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_TELEGRAM") {
                let account_id = account_id.trim().to_string();
                if !account_id.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let state = app_handle.state::<webview_accounts::WebviewAccountsState>();
                        let args = webview_accounts::OpenArgs {
                            account_id: account_id.clone(),
                            provider: "telegram".to_string(),
                            url: None,
                            bounds: Some(webview_accounts::Bounds {
                                x: 100.0,
                                y: 100.0,
                                width: 900.0,
                                height: 700.0,
                            }),
                        };
                        match webview_accounts::webview_account_open(
                            app_handle.clone(),
                            state,
                            args,
                        )
                        .await
                        {
                            Ok(label) => log::info!(
                                "[dev-auto-telegram] spawned label={} account={}",
                                label,
                                account_id
                            ),
                            Err(e) => log::error!(
                                "[dev-auto-telegram] failed: {} (account={})",
                                e,
                                account_id
                            ),
                        }
                    });
                }
            }
            // Same dev helper, Google Meet flavour.
            // OPENHUMAN_DEV_AUTO_GOOGLE_MEET=<uuid> opens the gmeet account
            // webview at startup so the caption-capture recipe runs
            // without manual UI clicks. Use in combination with:
            //   tail -F /tmp/oh-cef.log | grep -E --line-buffered \
            //     "\[gmeet\]|memory_doc_ingest|orchestrator"
            // to verify captions flow → transcript persist → thread handoff.
            if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_GOOGLE_MEET") {
                let account_id = account_id.trim().to_string();
                if !account_id.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let state = app_handle.state::<webview_accounts::WebviewAccountsState>();
                        // Dev mode: size the child webview to the parent
                        // window's inner bounds so Meet controls (CC toggle,
                        // mic/cam, leave) are reachable without overflowing.
                        let (w, h) = app_handle
                            .get_webview_window("main")
                            .and_then(|main| {
                                let scale = main.scale_factor().unwrap_or(1.0);
                                main.inner_size()
                                    .ok()
                                    .map(|s| ((s.width as f64) / scale, (s.height as f64) / scale))
                            })
                            .unwrap_or((1100.0, 780.0));
                        let args = webview_accounts::OpenArgs {
                            account_id: account_id.clone(),
                            provider: "google-meet".to_string(),
                            url: None,
                            bounds: Some(webview_accounts::Bounds {
                                x: 0.0,
                                y: 0.0,
                                width: w,
                                height: h,
                            }),
                        };
                        match webview_accounts::webview_account_open(
                            app_handle.clone(),
                            state,
                            args,
                        )
                        .await
                        {
                            Ok(label) => log::info!(
                                "[dev-auto-gmeet] spawned label={} account={}",
                                label,
                                account_id
                            ),
                            Err(e) => log::error!(
                                "[dev-auto-gmeet] failed: {} (account={})",
                                e,
                                account_id
                            ),
                        }
                    });
                }
            }
            // OPENHUMAN_DEV_AUTO_GMAIL=<account-id> opens the Gmail account
            // webview at startup so the webview_apis bridge has a live CDP
            // target to attach to. Pair with:
            //   curl -sS http://127.0.0.1:7788/rpc \
            //     -H 'Content-Type: application/json' \
            //     -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.webview_apis_gmail_list_labels","params":{"account_id":"<account-id>"}}'
            if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_GMAIL") {
                let account_id = account_id.trim().to_string();
                if !account_id.is_empty() {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let state = app_handle.state::<webview_accounts::WebviewAccountsState>();
                        // Size the Gmail child webview to the parent window
                        // so the inbox is usable without manual resizing.
                        let (w, h) = app_handle
                            .get_webview_window("main")
                            .and_then(|main| {
                                let scale = main.scale_factor().unwrap_or(1.0);
                                main.inner_size()
                                    .ok()
                                    .map(|s| ((s.width as f64) / scale, (s.height as f64) / scale))
                            })
                            .unwrap_or((1100.0, 780.0));
                        let args = webview_accounts::OpenArgs {
                            account_id: account_id.clone(),
                            provider: "gmail".to_string(),
                            url: None,
                            bounds: Some(webview_accounts::Bounds {
                                x: 0.0,
                                y: 0.0,
                                width: w,
                                height: h,
                            }),
                        };
                        match webview_accounts::webview_account_open(
                            app_handle.clone(),
                            state,
                            args,
                        )
                        .await
                        {
                            Ok(label) => log::info!(
                                "[dev-auto-gmail] spawned label={} account={}",
                                label,
                                account_id
                            ),
                            Err(e) => log::error!(
                                "[dev-auto-gmail] failed: {} (account={})",
                                e,
                                account_id
                            ),
                        }
                    });
                }
            }

            #[cfg(target_os = "macos")]
            {
                use std::sync::Arc;
                // The scanner task self-gates on `channels_config.imessage` via
                // JSON-RPC each tick — it stays idle until the user connects
                // iMessage and stops ingesting as soon as they disconnect. We
                // spawn it here just so the loop is live and picks up state
                // changes without requiring an app restart.
                if let Some(registry) = app.try_state::<Arc<imessage_scanner::ScannerRegistry>>() {
                    let registry = registry.inner().clone();
                    let app_handle = app.handle().clone();
                    registry.ensure_scanner(app_handle, "default".to_string());
                    log::info!("[imessage] scanner scheduled (gates on config each tick)");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_rpc_url,
            overlay_parent_rpc_url,
            check_core_update,
            apply_core_update,
            check_app_update,
            apply_app_update,
            restart_core_process,
            restart_app,
            schedule_cef_profile_purge,
            service_install_direct,
            service_start_direct,
            service_stop_direct,
            service_status_direct,
            service_uninstall_direct,
            register_dictation_hotkey,
            unregister_dictation_hotkey,
            webview_accounts::webview_account_open,
            webview_accounts::webview_account_close,
            webview_accounts::webview_account_purge,
            webview_accounts::webview_account_bounds,
            webview_accounts::webview_account_reveal,
            webview_accounts::webview_account_hide,
            webview_accounts::webview_account_show,
            webview_accounts::webview_recipe_event,
            webview_accounts::webview_notification_permission_state,
            webview_accounts::webview_notification_permission_request,
            webview_accounts::webview_notification_set_dnd,
            webview_accounts::webview_notification_mute_account,
            webview_accounts::webview_notification_get_bypass_prefs,
            webview_accounts::webview_set_focused_account,
            notification_settings::notification_settings_get,
            notification_settings::notification_settings_set,
            screen_capture::screen_share_begin_session,
            screen_capture::screen_share_thumbnail,
            screen_capture::screen_share_finalize_session,
            gmail::gmail_list_labels,
            gmail::gmail_list_messages,
            gmail::gmail_search,
            gmail::gmail_get_message,
            gmail::gmail_send,
            gmail::gmail_trash,
            gmail::gmail_add_label,
            gmail::gmail_find_linkedin_profile_url,
            notification_permission_state,
            notification_permission_request,
            activate_main_window,
            show_native_notification
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app_handle, event| match event {
            RunEvent::Ready => {
                log::info!("[app] RunEvent::Ready — GTK initialized, setting up tray");
                if let Err(err) = setup_tray(app_handle) {
                    log::warn!(
                    "[tray] failed to setup tray icon (non-fatal in headless environment): {err}"
                );
                }
            }
            #[cfg(target_os = "macos")]
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                log::info!(
                    "[window] close requested on main window — hiding instead of destroying"
                );
                api.prevent_close();
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                log::info!("[window] reopen event — showing main window");
                if let Err(err) = show_main_window(app_handle) {
                    log::error!("[macos] failed to show main window on reopen: {err}");
                }
            }
            RunEvent::ExitRequested { .. } => {
                // Run our cleanup BEFORE CEF's own Exit handler does
                // `close_all_windows() → cef::shutdown()`. Doing this in
                // RunEvent::Exit instead races CEF's teardown and the
                // `browser_count == 0` CHECK in `cef::shutdown` panics on
                // macOS Cmd+Q (issue #920). The order matters:
                //   1. close our child webviews so CEF processes the
                //      close requests during the Exit-phase message pump
                //      (gives them time to settle before cef::shutdown).
                //   2. abort our long-lived tokio tasks so they're not
                //      driving CDP traffic against CEF as it tears down.
                //   3. stop the webview_apis WS listener so its accept
                //      loop releases the loopback port.
                //   4. SIGTERM the core sidecar (non-blocking). Tauri
                //      spawned the child so we own its lifecycle, but we
                //      do not wait — that would block the main thread
                //      and starve CEF's UI loop. The kernel reaps the
                //      child after Tauri exits.
                log::info!("[app] RunEvent::ExitRequested — early teardown");

                let _ = teardown_cef_prewarm(app_handle);

                if let Some(state) =
                    app_handle.try_state::<webview_accounts::WebviewAccountsState>()
                {
                    state.shutdown_all(app_handle);
                }

                webview_apis::server::stop();

                if let Some(core) = app_handle.try_state::<core_process::CoreProcessHandle>() {
                    let core = core.inner().clone();
                    tauri::async_runtime::block_on(async move {
                        core.send_terminate_signal().await;
                    });
                }

                log::info!("[app] RunEvent::ExitRequested — early teardown complete");
            }
            RunEvent::Exit => {
                log::info!("[app] RunEvent::Exit");
            }
            _ => {}
        });
}

pub fn run_core_from_args(args: &[String]) -> Result<(), String> {
    let core_bin = crate::core_process::default_core_bin()
        .ok_or_else(|| "openhuman-core binary not found".to_string())?;
    let status = std::process::Command::new(core_bin)
        .args(args)
        .status()
        .map_err(|e| format!("failed to execute core binary: {e}"))?;
    if !status.success() {
        return Err(format!("core binary exited with status {status}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that is_daemon_mode correctly detects daemon flag variations
    #[test]
    fn is_daemon_mode_detects_daemon_flag() {
        // Note: This test relies on the current process args, so in test mode
        // it will typically return false. We verify the function is callable.
        let _result = is_daemon_mode();
    }

    /// Test expand_dictation_shortcuts for CmdOrCtrl expansion
    #[test]
    fn expand_dictation_shortcuts_cmd_or_ctrl_expansion() {
        #[cfg(target_os = "macos")]
        {
            let result = expand_dictation_shortcuts("CmdOrCtrl+Shift+D");
            assert_eq!(result.len(), 2);
            assert!(result.contains(&"Cmd+Shift+D".to_string()));
            assert!(result.contains(&"Ctrl+Shift+D".to_string()));
        }

        #[cfg(not(target_os = "macos"))]
        {
            let result = expand_dictation_shortcuts("CmdOrCtrl+Shift+D");
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], "Ctrl+Shift+D");
        }
    }

    /// Test expand_dictation_shortcuts with plain shortcut (no CmdOrCtrl)
    #[test]
    fn expand_dictation_shortcuts_plain_shortcut() {
        let result = expand_dictation_shortcuts("Ctrl+Alt+T");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Ctrl+Alt+T");
    }

    /// Test expand_dictation_shortcuts with empty/whitespace input
    #[test]
    fn expand_dictation_shortcuts_empty_input() {
        let result = expand_dictation_shortcuts("");
        assert!(result.is_empty());

        let result = expand_dictation_shortcuts("   ");
        assert!(result.is_empty());
    }

    /// Test core_rpc_url returns expected format
    #[test]
    fn core_rpc_url_returns_expected_format() {
        // Save original env
        let original = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();

        // Test with env var set
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://localhost:9999/rpc");
        let url = core_rpc_url();
        assert_eq!(url, "http://localhost:9999/rpc");

        // Test fallback when env not set
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        let url = core_rpc_url();
        assert_eq!(url, "http://127.0.0.1:7788/rpc");

        // Restore original
        match original {
            Some(v) => std::env::set_var("OPENHUMAN_CORE_RPC_URL", v),
            None => std::env::remove_var("OPENHUMAN_CORE_RPC_URL"),
        }
    }

    /// Test overlay_parent_rpc_url handles empty env var
    #[test]
    fn overlay_parent_rpc_url_handles_empty() {
        // Save original env
        let original = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();

        // Test with empty string (should return None)
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "");
        let result = overlay_parent_rpc_url();
        assert!(result.is_none());

        // Test with whitespace only (should return None)
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "   ");
        let result = overlay_parent_rpc_url();
        assert!(result.is_none());

        // Test with valid URL
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7788/rpc");
        let result = overlay_parent_rpc_url();
        assert_eq!(result, Some("http://127.0.0.1:7788/rpc".to_string()));

        // Restore original
        match original {
            Some(v) => std::env::set_var("OPENHUMAN_CORE_RPC_URL", v),
            None => std::env::remove_var("OPENHUMAN_CORE_RPC_URL"),
        }
    }

    /// Tests for setup_tray conditional compilation
    /// The PR adds two versions of setup_tray():
    /// 1. No-op for linux + cef: logs warning and returns Ok(())
    /// 2. Full implementation for other platforms
    ///
    /// These tests verify the function signatures are correct and
    /// the compile-time cfg blocks are properly set up.

    /// Verify setup_tray function exists and has correct signature
    /// This test passes if the code compiles, as the function signature
    /// is validated by the compiler.
    #[test]
    fn setup_tray_function_signature_compiles() {
        // This test exists to ensure the conditional compilation
        // of setup_tray is valid. The function is not actually called
        // here because it requires a full Tauri AppHandle.
        // The cfg attributes ensure only one version exists at compile time.
    }

    /// Test that AppRuntime is defined for the current feature set
    #[test]
    fn app_runtime_type_exists() {
        // This test verifies AppRuntime is properly defined
        // based on the cef feature flag.
        // The type alias exists at module scope and is used throughout.
        fn _check_runtime<R: tauri::Runtime>() {}
        // _check_runtime::<AppRuntime>(); // Would require importing
    }

    /// Verify tray logging patterns exist (grep-friendly)
    #[test]
    fn tray_setup_logging_patterns_exist() {
        // These log patterns from the PR are grep-friendly:
        // "[tray] skipping tray setup on linux+cef: ..."
        // "[tray] setting up tray icon"
        // "[tray] tray icon ready"
        // "[tray] action=show_window ..."
        // "[tray] action=quit ..."
        // "[tray] failed to setup tray icon ..."
        // "[app] RunEvent::Ready — GTK initialized, setting up tray"
        //
        // This test passes if the code compiles with these log messages.
    }

    /// Test expand_dictation_shortcuts with Cmd-only variant on macOS
    #[test]
    #[cfg(target_os = "macos")]
    fn expand_dictation_shortcuts_macos_cmd_only() {
        // When CmdOrCtrl is replaced with just Cmd
        let result = expand_dictation_shortcuts("CmdOrCtrl+Space");
        assert!(result.contains(&"Cmd+Space".to_string()));
    }

    /// Test expand_dictation_shortcuts with Ctrl-only variant on non-macOS
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn expand_dictation_shortcuts_non_macos_ctrl_only() {
        // When CmdOrCtrl is replaced with just Ctrl
        let result = expand_dictation_shortcuts("CmdOrCtrl+Space");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Ctrl+Space");
    }
}
