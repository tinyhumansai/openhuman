//! AlphaHuman Desktop Application
//!
//! This is the Rust backend for the cross-platform crypto community platform.
//! It provides:
//! - System tray with background execution
//! - Deep link authentication
//! - Persistent Socket.io connection
//! - Secure session storage
//! - Native notifications

mod commands;
mod models;
mod services;
mod utils;

use commands::*;
use services::socket_service::SOCKET_SERVICE;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent,
};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

/// Demo command - can be removed in production
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Helper function to show the window
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// Helper function to toggle window visibility
fn toggle_main_window_visibility(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        match window.is_visible() {
            Ok(true) => {
                let _ = window.hide();
            }
            Ok(false) => {
                show_main_window(app);
            }
            Err(_) => {
                // If we can't determine visibility, try to show it
                show_main_window(app);
            }
        }
    } else {
        eprintln!("Could not find window 'main'");
    }
}

// Setup system tray with menu
#[cfg(desktop)]
fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let show_hide_item = MenuItem::with_id(app, "show_hide", "Show/Hide Window", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_hide_item, &quit_item])?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("AlphaHuman")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show_hide" => {
                toggle_main_window_visibility(app);
            }
            "quit" => {
                // Cleanup before exit - request frontend to disconnect
                let _ = SOCKET_SERVICE.request_disconnect();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                let app = tray.app_handle();
                toggle_main_window_visibility(app);
            }
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                let app = tray.app_handle();
                show_main_window(app);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ));

    // Add notification plugin on desktop only
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_notification::init());
    }

    builder
        // Setup
        .setup(|app| {
            // Initialize socket service with app handle
            SOCKET_SERVICE.set_app_handle(app.handle().clone());

            // Register deep link handlers (Windows/Linux)
            #[cfg(any(windows, target_os = "linux"))]
            {
                app.deep_link().register_all()?;
            }

            // Setup system tray (desktop only)
            #[cfg(desktop)]
            {
                setup_tray(app.handle())?;
            }

            // macOS-specific: Handle window close event to minimize to tray
            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let app_handle = app.handle().clone();
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            // Prevent the window from closing, hide it instead
                            api.prevent_close();
                            if let Some(win) = app_handle.get_webview_window("main") {
                                let _ = win.hide();
                            }
                        }
                    });
                }
            }

            Ok(())
        })
        // Register all commands
        .invoke_handler(tauri::generate_handler![
            // Demo
            greet,
            // Auth commands
            exchange_token,
            get_auth_state,
            get_session_token,
            get_current_user,
            is_authenticated,
            logout,
            store_session,
            // Socket commands
            socket_connect,
            socket_disconnect,
            get_socket_state,
            is_socket_connected,
            report_socket_connected,
            report_socket_disconnected,
            report_socket_error,
            update_socket_status,
            // Telegram commands
            start_telegram_login,
            start_telegram_login_with_url,
            // Window commands
            show_window,
            hide_window,
            toggle_window,
            is_window_visible,
            minimize_window,
            maximize_window,
            close_window,
            set_window_title,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Handle macOS Dock icon click (reopen event)
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                show_main_window(app_handle);
            }

            // Suppress unused variable warning on non-macOS
            #[cfg(not(target_os = "macos"))]
            let _ = (app_handle, event);
        });
}
