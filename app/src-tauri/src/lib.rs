#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host is desktop-only. Non-desktop targets are not supported.");

mod core_process;
mod core_update;
#[cfg(feature = "cef")]
mod discord_scanner;
#[cfg(feature = "cef")]
mod imessage_scanner;
mod notification_settings;
#[cfg(feature = "cef")]
mod slack_scanner;
#[cfg(feature = "cef")]
mod telegram_scanner;
mod webview_accounts;
mod whatsapp_scanner;

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject};
#[cfg(target_os = "macos")]
use objc2::ClassType;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSPanel, NSWindowCollectionBehavior, NSWindowStyleMask};

// Runtime backend alias. `AppHandle`/`WebviewWindow` get their default `R`
// (the runtime generic) from whichever runtime feature the tauri crate has
// enabled — and when we drop `tauri/wry` in favor of `tauri/cef`, `Wry`
// disappears entirely, so plain `AppHandle` (no generic) stops resolving.
// Every helper that touches an `AppHandle` or `WebviewWindow` threads this
// alias through its signature; tauri command handlers get the right runtime
// automatically from the `#[tauri::command]` macro.
#[cfg(feature = "cef")]
pub(crate) type AppRuntime = tauri::Cef;
#[cfg(not(feature = "cef"))]
pub(crate) type AppRuntime = tauri::Wry;

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

pub fn run() {
    let daemon_mode = is_daemon_mode();

    let default_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = env_logger::Builder::new()
        .parse_filters(&default_filter)
        .try_init();

    // Runtime selection: default build uses wry (WKWebView on macOS), the
    // `cef` feature swaps to Chromium Embedded Framework. The switch is at
    // Builder construction only — everything downstream (plugins, commands,
    // events, state, tray, deep links, child webviews) uses the shared
    // tauri-runtime trait surface and does not care which backend drives it.
    #[cfg(not(feature = "cef"))]
    let builder = tauri::Builder::<tauri::Wry>::new();
    #[cfg(feature = "cef")]
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
        // Protocol on localhost:9222 so every CEF webview can be inspected
        // from a regular browser (right-click "Inspect" does not propagate
        // to CEF child webviews on macOS). Release builds intentionally do
        // NOT open the CDP port — it would let any process on the machine
        // drive the embedded WhatsApp/Slack/etc. webviews.
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
        if cfg!(debug_assertions) {
            args.push(("--remote-debugging-port", Some("9222")));
        }
        tauri::Builder::<tauri::Cef>::new().command_line_args::<&str, &str>(args)
    };

    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(DictationHotkeyState(Mutex::new(Vec::new())))
        .manage(webview_accounts::WebviewAccountsState::default())
        .manage(notification_settings::NotificationSettingsState::new());
    #[cfg(feature = "cef")]
    let builder = builder.manage(std::sync::Arc::new(imessage_scanner::ScannerRegistry::new()));
    let builder = builder.manage(whatsapp_scanner::ScannerRegistry::new());
    #[cfg(feature = "cef")]
    let builder = builder.manage(slack_scanner::ScannerRegistry::new());
    #[cfg(feature = "cef")]
    let builder = builder.manage(discord_scanner::ScannerRegistry::new());
    #[cfg(feature = "cef")]
    let builder = builder.manage(telegram_scanner::ScannerRegistry::new());
    builder
        .setup(move |app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                if let Err(err) = app.deep_link().register_all() {
                    log::warn!("[deep-link] register_all failed (non-fatal): {err}");
                }
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

            if let Err(err) = setup_tray(app.handle()) {
                log::error!("[tray] failed to setup tray icon: {err}");
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

            #[cfg(all(target_os = "macos", feature = "cef"))]
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
            restart_core_process,
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
            webview_accounts::webview_account_hide,
            webview_accounts::webview_account_show,
            webview_accounts::webview_recipe_event,
            webview_accounts::webview_account_eval,
            notification_settings::notification_settings_get,
            notification_settings::notification_settings_set,
            activate_main_window
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app_handle, event| match event {
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
            RunEvent::Exit => {
                if let Some(core) = app_handle.try_state::<core_process::CoreProcessHandle>() {
                    let core = core.inner().clone();
                    tauri::async_runtime::block_on(async move {
                        core.shutdown().await;
                    });
                }
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
