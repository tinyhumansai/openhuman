#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host is desktop-only. Non-desktop targets are not supported.");

mod core_process;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent,
};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

#[tauri::command]
fn core_rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
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

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

fn show_main_window(app: &AppHandle) {
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

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                app.deep_link().register_all()?;
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
            tauri::async_runtime::spawn(async move {
                if let Err(err) = core_handle.ensure_running().await {
                    log::error!("[core] failed to start core process: {err}");
                } else {
                    log::info!("[core] core process ready");
                }
            });

            if daemon_mode {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    log::info!("[tray] daemon_mode=true window_hidden_on_startup");
                }
            }

            if let Err(err) = setup_tray(app.handle()) {
                log::error!("[tray] failed to setup tray icon: {err}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_rpc_url,
            service_install_direct,
            service_start_direct,
            service_stop_direct,
            service_status_direct,
            service_uninstall_direct
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
