//! Core-owned desktop tray integration for OpenHuman host processes.

mod schemas;
pub use schemas::{
    all_controller_schemas as all_tray_controller_schemas,
    all_registered_controllers as all_tray_registered_controllers,
};

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("Tray support is desktop-only.");

use tauri::AppHandle;
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use tauri::Manager;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

/// Show and focus the main app window.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

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
                show_main_window(app);
            }
        }
    } else {
        log::warn!("[tray] Main window not found");
    }
}

/// Build and register the system tray icon/menu.
pub fn setup_tray(app: &AppHandle) -> Result<(), String> {
    let app_for_menu = app.clone();
    let app_for_tray = app.clone();

    let show_hide_item =
        MenuItem::with_id(app, "show_hide", "Show/Hide Window", true, None::<&str>)
            .map_err(|e| format!("failed to create tray menu item: {e}"))?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
        .map_err(|e| format!("failed to create tray quit item: {e}"))?;

    let menu = Menu::with_items(app, &[&show_hide_item, &quit_item])
        .map_err(|e| format!("failed to build tray menu: {e}"))?;

    let icon = app
        .default_window_icon()
        .ok_or_else(|| "default window icon is unavailable".to_string())?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon.clone())
        .menu(&menu)
        .tooltip("OpenHuman")
        .on_menu_event(move |_app, event| match event.id().as_ref() {
            "show_hide" => {
                toggle_main_window_visibility(&app_for_menu);
            }
            "quit" => {
                app_for_menu.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(move |_tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                toggle_main_window_visibility(&app_for_tray);
            }
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                show_main_window(&app_for_tray);
            }
            _ => {}
        })
        .build(app)
        .map_err(|e| format!("failed to build tray icon: {e}"))?;

    Ok(())
}
