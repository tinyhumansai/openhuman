#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host is desktop-only. Non-desktop targets are not supported.");

#[cfg(feature = "cef")]
mod whatsapp_scanner;
mod core_process;
mod core_update;
mod webview_accounts;

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
#[cfg(target_os = "macos")]
use objc2_core_graphics::CGShieldingWindowLevel;

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
fn configure_overlay_window_macos(window: &WebviewWindow<AppRuntime>) {
    if let Err(err) = window.set_always_on_top(true) {
        log::warn!("[overlay] failed to set always-on-top: {err}");
    }
    if let Err(err) = window.set_visible_on_all_workspaces(true) {
        log::warn!("[overlay] failed to set visible-on-all-workspaces: {err}");
    }

    match window.ns_window() {
        Ok(ns_window) => unsafe {
            let window: &NSWindow = &*ns_window.cast();
            let mut behavior = window.collectionBehavior();
            behavior.insert(NSWindowCollectionBehavior::FullScreenAuxiliary);
            behavior.insert(NSWindowCollectionBehavior::CanJoinAllSpaces);
            window.setCollectionBehavior(behavior);
            window.setLevel((CGShieldingWindowLevel() + 1) as isize);
            log::info!(
                "[overlay] macOS overlay configured for all spaces/fullscreen auxiliary at shielding+1 level"
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
async fn register_dictation_hotkey(app: AppHandle<AppRuntime>, shortcut: String) -> Result<(), String> {
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

fn show_main_window(app: &AppHandle<AppRuntime>) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(err) = window.show() {
            log::error!("[tray] failed to show main window: {err}");
        }
        if let Err(err) = window.unminimize() {
            log::error!("[tray] failed to unminimize main window: {err}");
        }
        if let Err(err) = window.set_focus() {
            log::error!("[tray] failed to focus main window: {err}");
        }
    } else {
        log::error!("[tray] main window not found");
    }
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
                show_main_window(app);
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
                show_main_window(tray.app_handle());
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
    let builder = tauri::Builder::<tauri::Cef>::new()
        // Bypass macOS Keychain. Without this, every embedded service that
        // touches password / cookie / encryption-key storage triggers a
        // "Allow access to your keychain?" prompt — WhatsApp Web hits it on
        // every cold start, Chromium's own component-update store also does.
        // `use-mock-keychain` swaps the Keychain backend for an in-process
        // mock; `password-store=basic` is the equivalent for the password
        // manager. Both are no-ops on Windows/Linux, so safe to always set.
        //
        // `remote-debugging-port` exposes Chrome DevTools for every CEF
        // webview (main window + per-account service views) at
        //   http://localhost:9222
        // — open that URL in any browser to pick a target. Right-click
        // "Inspect" does not work on CEF child webviews on macOS, so this
        // is the only reliable way to inspect IndexedDB / console / storage
        // for the embedded WhatsApp/Slack/etc. webviews.
        // NOTE: flags must be prefixed with `--`. The runtime's
        // `on_before_command_line_processing` dispatch (in
        // `tauri-runtime-cef/src/cef_impl.rs`) routes value-less args that
        // don't start with `-` to `append_argument` (positional) instead of
        // `append_switch`, which means Chromium silently ignores them.
        .command_line_args::<&str, &str>([
            ("--use-mock-keychain", None),
            ("--password-store", Some("basic")),
            ("--remote-debugging-port", Some("9222")),
            ("--remote-allow-origins", Some("*")),
        ]);

    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(DictationHotkeyState(Mutex::new(Vec::new())))
        .manage(webview_accounts::WebviewAccountsState::default());
    #[cfg(feature = "cef")]
    let builder = builder.manage(whatsapp_scanner::ScannerRegistry::new());
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

            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("overlay") {
                    configure_overlay_window_macos(&window);
                } else {
                    log::warn!("[overlay] overlay window not found during setup");
                }
            }

            if let Some(window) = app.get_webview_window("overlay") {
                pin_overlay_bottom_right(&window);
                if let Err(err) = window.show() {
                    log::warn!("[overlay] failed to show overlay on startup: {err}");
                } else {
                    log::info!("[overlay] overlay shown on startup");
                }
            }

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
                        let state = app_handle
                            .state::<webview_accounts::WebviewAccountsState>();
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
            webview_accounts::webview_account_bounds,
            webview_accounts::webview_account_hide,
            webview_accounts::webview_account_show,
            webview_accounts::webview_recipe_event,
            webview_accounts::webview_account_set_suggestion,
            webview_accounts::webview_account_clear_suggestion,
            webview_accounts::webview_account_commit_suggestion,
            webview_accounts::webview_account_eval
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app_handle, event| match event {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                show_main_window(app_handle);
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
