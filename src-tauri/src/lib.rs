//! AlphaHuman Desktop Application
//!
//! This is the Rust backend for the cross-platform crypto community platform.
//! It provides:
//! - System tray with background execution
//! - Deep link authentication
//! - Persistent Socket.io connection
//! - Secure session storage
//! - Native notifications

mod ai;
mod commands;
mod models;
mod runtime;
mod services;
mod utils;

use ai::*;
use commands::*;
use services::socket_service::SOCKET_SERVICE;
use tauri::{AppHandle, Manager, RunEvent};

#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

/// Demo command - can be removed in production
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Macro to define common handlers shared across all platforms
macro_rules! common_handlers {
    () => {
        // Demo
        greet,
        // Auth commands
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
        // AI encryption commands
        ai_init_encryption,
        ai_encrypt,
        ai_decrypt,
        // AI memory filesystem commands
        ai_memory_init,
        ai_memory_upsert_file,
        ai_memory_get_file,
        ai_memory_upsert_chunk,
        ai_memory_delete_chunks_by_path,
        ai_memory_fts_search,
        ai_memory_get_chunks,
        ai_memory_get_all_embeddings,
        ai_memory_cache_embedding,
        ai_memory_get_cached_embedding,
        ai_memory_set_meta,
        ai_memory_get_meta,
        // AI session commands
        ai_sessions_init,
        ai_sessions_load_index,
        ai_sessions_update_index,
        ai_sessions_append_transcript,
        ai_sessions_read_transcript,
        ai_sessions_delete,
        ai_sessions_list,
        ai_read_memory_file,
        ai_write_memory_file,
        ai_list_memory_files,
        // Runtime commands
        runtime_discover_skills,
        runtime_list_skills,
        runtime_start_skill,
        runtime_stop_skill,
        runtime_get_skill_state,
        runtime_call_tool,
        runtime_all_tools,
        runtime_broadcast_event,
        // Runtime enable/disable + KV commands
        runtime_enable_skill,
        runtime_disable_skill,
        runtime_is_skill_enabled,
        runtime_get_skill_preferences,
        runtime_skill_kv_get,
        runtime_skill_kv_set,
        // Runtime JSON-RPC + data commands
        runtime_rpc,
        runtime_skill_data_read,
        runtime_skill_data_write,
        runtime_skill_data_dir,
        // Socket.io commands (Rust-native persistent connection)
        runtime_socket_connect,
        runtime_socket_disconnect,
        runtime_socket_state,
        runtime_socket_emit,
    };
}

// Macro to define desktop-only window handlers
macro_rules! desktop_window_handlers {
    () => {
        show_window,
        hide_window,
        toggle_window,
        is_window_visible,
        minimize_window,
        maximize_window,
        close_window,
        set_window_title,
    };
}

// Helper function to show the window (used by tray and macOS reopen)
#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// Helper function to toggle window visibility
#[cfg(desktop)]
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
    let show_hide_item =
        MenuItem::with_id(app, "show_hide", "Show/Hide Window", true, None::<&str>)?;
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

    // Initialize platform-appropriate logger
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("AlphaHuman"),
        );
    }
    #[cfg(not(target_os = "android"))]
    {
        use env_logger::fmt::style::{AnsiColor, Style};
        use std::io::Write;

        let default_filter = std::env::var("RUST_LOG")
            .unwrap_or_else(|_| "info,tungstenite=warn,tokio_tungstenite=warn,reqwest=warn,rusqlite=warn,hyper=warn,h2=warn".to_string());

        let write_style = std::env::var("RUST_LOG_STYLE")
            .map(|v| match v.as_str() {
                "never" => env_logger::fmt::WriteStyle::Never,
                _ => env_logger::fmt::WriteStyle::Always,
            })
            .unwrap_or(env_logger::fmt::WriteStyle::Always);

        let _ = env_logger::Builder::new()
            .parse_filters(&default_filter)
            .write_style(write_style)
            .format(|buf, record| {
                let timestamp = buf.timestamp_millis()
                    .to_string();
                // Strip the date prefix, keep only HH:MM:SS.mmm
                let time_only = timestamp.split('T')
                    .nth(1)
                    .and_then(|t| t.strip_suffix('Z'))
                    .unwrap_or(&timestamp);
                let level = record.level();

                // Level colors
                let level_style = match level {
                    log::Level::Error => Style::new().fg_color(Some(AnsiColor::Red.into())).bold(),
                    log::Level::Warn  => Style::new().fg_color(Some(AnsiColor::Yellow.into())).bold(),
                    log::Level::Info  => Style::new().fg_color(Some(AnsiColor::Green.into())),
                    log::Level::Debug => Style::new().fg_color(Some(AnsiColor::BrightBlack.into())),
                    log::Level::Trace => Style::new().fg_color(Some(AnsiColor::BrightBlack.into())),
                };

                let msg = format!("{}", record.args());

                // Extract tag from message (e.g. "[tdlib]", "[socket-mgr]", "[skill:x]")
                let (tag, rest) = if msg.starts_with('[') {
                    if let Some(end) = msg.find(']') {
                        let tag = &msg[..=end];
                        let rest = msg[end + 1..].trim_start();
                        (Some(tag.to_string()), rest.to_string())
                    } else {
                        (None, msg)
                    }
                } else {
                    (None, msg)
                };

                // Tag-based colors
                let tag_style = if let Some(ref t) = tag {
                    let t_lower = t.to_lowercase();
                    if t_lower.contains("tdlib") {
                        Style::new().fg_color(Some(AnsiColor::Magenta.into())).bold()
                    } else if t_lower.contains("socket") {
                        Style::new().fg_color(Some(AnsiColor::Blue.into())).bold()
                    } else if t_lower.contains("runtime") {
                        Style::new().fg_color(Some(AnsiColor::Cyan.into())).bold()
                    } else if t_lower.contains("skill") {
                        Style::new().fg_color(Some(AnsiColor::Green.into())).bold()
                    } else if t_lower.contains("ping") || t_lower.contains("cron") {
                        Style::new().fg_color(Some(AnsiColor::Yellow.into())).bold()
                    } else if t_lower.contains("app") {
                        Style::new().fg_color(Some(AnsiColor::White.into())).bold()
                    } else if t_lower.contains("ai") {
                        Style::new().fg_color(Some(AnsiColor::BrightMagenta.into())).bold()
                    } else {
                        Style::new().fg_color(Some(AnsiColor::BrightBlack.into()))
                    }
                } else {
                    Style::new()
                };

                let dim = Style::new().fg_color(Some(AnsiColor::BrightBlack.into()));

                if let Some(ref t) = tag {
                    writeln!(
                        buf,
                        "{dim}{time_only}{dim:#} {level_style}{level:<5}{level_style:#} {tag_style}{t}{tag_style:#} {rest}"
                    )
                } else {
                    writeln!(
                        buf,
                        "{dim}{time_only}{dim:#} {level_style}{level:<5}{level_style:#} {rest}"
                    )
                }
            })
            .try_init();
    }

    let mut builder = tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_os::init());

    // Add desktop-only plugins (autostart, notification)
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec!["--minimized"]),
            ))
            .plugin(tauri_plugin_notification::init());
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

            // Create the SocketManager (persistent Rust-native Socket.io)
            let socket_mgr = std::sync::Arc::new(
                runtime::socket_manager::SocketManager::new(),
            );
            socket_mgr.set_app_handle(app.handle().clone());

            // Initialize QuickJS Runtime Engine (desktop only - not available on Android/iOS)
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| {
                        // Fallback for platforms where app_data_dir isn't available
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".alphahuman")
                    });
                let skills_data_dir = data_dir.join("skills");

                match runtime::qjs_engine::RuntimeEngine::new(skills_data_dir) {
                    Ok(engine) => {
                        engine.set_app_handle(app.handle().clone());

                        // Set resource directory for bundled skills (production builds)
                        if let Ok(resource_dir) = app.path().resource_dir() {
                            engine.set_resource_dir(resource_dir);
                        }

                        let engine = std::sync::Arc::new(engine);

                        // Wire the SkillRegistry into the SocketManager for MCP
                        socket_mgr.set_registry(engine.registry());
                        // Wire the SocketManager into the engine for tool:sync
                        engine.set_socket_manager(socket_mgr.clone());

                        app.manage(engine.clone());

                        // Start the cron scheduler
                        let cron = engine.cron_scheduler();
                        tauri::async_runtime::spawn(async move {
                            cron.start();
                        });

                        // Start the ping scheduler (health-checks running skills)
                        let ping = engine.ping_scheduler();
                        tauri::async_runtime::spawn(async move {
                            ping.start();
                        });

                        // Auto-start skills in background (no delay needed for QuickJS -
                        // lightweight contexts don't have V8's memory reservation issue)
                        let engine_clone = engine.clone();
                        tauri::async_runtime::spawn(async move {
                            engine_clone.auto_start_skills().await;
                        });

                        log::info!("[runtime] QuickJS runtime engine initialized");
                    }
                    Err(e) => {
                        log::error!("[runtime] Failed to initialize QuickJS runtime: {e}");
                    }
                }
            }

            #[cfg(target_os = "android")]
            {
                log::info!("[runtime] QuickJS runtime disabled on Android");
            }

            #[cfg(target_os = "ios")]
            {
                log::info!("[runtime] QuickJS runtime disabled on iOS");
            }

            // Store SocketManager as Tauri state
            app.manage(socket_mgr.clone());

            // Auto-connect socket if there's an existing session
            let socket_mgr_clone = socket_mgr.clone();
            tauri::async_runtime::spawn(async move {
                use commands::auth::SESSION_SERVICE;
                if let Some(token) = SESSION_SERVICE.get_token() {
                    let url = utils::config::get_backend_url();
                    log::info!("[socket-mgr] Auto-connecting with stored session");
                    if let Err(e) = socket_mgr_clone.connect(&url, &token).await {
                        log::error!("[socket-mgr] Auto-connect failed: {e}");
                    }
                } else {
                    log::info!("[socket-mgr] No stored session — waiting for login");
                }
            });

            Ok(())
        })
        // Register all commands
        // Common handlers are defined via macros above, conditionally include desktop window handlers
        .invoke_handler({
            #[cfg(desktop)]
            {
                tauri::generate_handler![
                    // Common handlers (expanded from common_handlers! macro)
                    greet,
                    get_auth_state,
                    get_session_token,
                    get_current_user,
                    is_authenticated,
                    logout,
                    store_session,
                    socket_connect,
                    socket_disconnect,
                    get_socket_state,
                    is_socket_connected,
                    report_socket_connected,
                    report_socket_disconnected,
                    report_socket_error,
                    update_socket_status,
                    // Desktop-only window handlers (expanded from desktop_window_handlers! macro)
                    show_window,
                    hide_window,
                    toggle_window,
                    is_window_visible,
                    minimize_window,
                    maximize_window,
                    close_window,
                    set_window_title,
                    // AI encryption commands
                    ai_init_encryption,
                    ai_encrypt,
                    ai_decrypt,
                    // AI memory filesystem commands
                    ai_memory_init,
                    ai_memory_upsert_file,
                    ai_memory_get_file,
                    ai_memory_upsert_chunk,
                    ai_memory_delete_chunks_by_path,
                    ai_memory_fts_search,
                    ai_memory_get_chunks,
                    ai_memory_get_all_embeddings,
                    ai_memory_cache_embedding,
                    ai_memory_get_cached_embedding,
                    ai_memory_set_meta,
                    ai_memory_get_meta,
                    // AI session commands
                    ai_sessions_init,
                    ai_sessions_load_index,
                    ai_sessions_update_index,
                    ai_sessions_append_transcript,
                    ai_sessions_read_transcript,
                    ai_sessions_delete,
                    ai_sessions_list,
                    ai_read_memory_file,
                    ai_write_memory_file,
                    ai_list_memory_files,
                    // Runtime commands
                    runtime_discover_skills,
                    runtime_list_skills,
                    runtime_start_skill,
                    runtime_stop_skill,
                    runtime_get_skill_state,
                    runtime_call_tool,
                    runtime_all_tools,
                    runtime_broadcast_event,
                    // Runtime enable/disable + KV commands
                    runtime_enable_skill,
                    runtime_disable_skill,
                    runtime_is_skill_enabled,
                    runtime_get_skill_preferences,
                    runtime_skill_kv_get,
                    runtime_skill_kv_set,
                    // Runtime JSON-RPC + data commands
                    runtime_rpc,
                    runtime_skill_data_read,
                    runtime_skill_data_write,
                    runtime_skill_data_dir,
                    // Socket.io commands (Rust-native persistent connection)
                    runtime_socket_connect,
                    runtime_socket_disconnect,
                    runtime_socket_state,
                    runtime_socket_emit,
                    // TDLib commands (native Telegram library)
                    tdlib_create_client,
                    tdlib_send,
                    tdlib_receive,
                    tdlib_destroy,
                    tdlib_is_available,
                    // Model commands (backend API proxy)
                    model_summarize,
                    model_generate,
                ]
            }
            #[cfg(not(desktop))]
            {
                tauri::generate_handler![
                    // Common handlers (expanded from common_handlers! macro)
                    greet,
                    get_auth_state,
                    get_session_token,
                    get_current_user,
                    is_authenticated,
                    logout,
                    store_session,
                    socket_connect,
                    socket_disconnect,
                    get_socket_state,
                    is_socket_connected,
                    report_socket_connected,
                    report_socket_disconnected,
                    report_socket_error,
                    update_socket_status,
                    // AI encryption commands
                    ai_init_encryption,
                    ai_encrypt,
                    ai_decrypt,
                    // AI memory filesystem commands
                    ai_memory_init,
                    ai_memory_upsert_file,
                    ai_memory_get_file,
                    ai_memory_upsert_chunk,
                    ai_memory_delete_chunks_by_path,
                    ai_memory_fts_search,
                    ai_memory_get_chunks,
                    ai_memory_get_all_embeddings,
                    ai_memory_cache_embedding,
                    ai_memory_get_cached_embedding,
                    ai_memory_set_meta,
                    ai_memory_get_meta,
                    // AI session commands
                    ai_sessions_init,
                    ai_sessions_load_index,
                    ai_sessions_update_index,
                    ai_sessions_append_transcript,
                    ai_sessions_read_transcript,
                    ai_sessions_delete,
                    ai_sessions_list,
                    ai_read_memory_file,
                    ai_write_memory_file,
                    ai_list_memory_files,
                    // Runtime commands
                    runtime_discover_skills,
                    runtime_list_skills,
                    runtime_start_skill,
                    runtime_stop_skill,
                    runtime_get_skill_state,
                    runtime_call_tool,
                    runtime_all_tools,
                    runtime_broadcast_event,
                    // Runtime enable/disable + KV commands
                    runtime_enable_skill,
                    runtime_disable_skill,
                    runtime_is_skill_enabled,
                    runtime_get_skill_preferences,
                    runtime_skill_kv_get,
                    runtime_skill_kv_set,
                    // Runtime JSON-RPC + data commands
                    runtime_rpc,
                    runtime_skill_data_read,
                    runtime_skill_data_write,
                    runtime_skill_data_dir,
                    // Socket.io commands (Rust-native persistent connection)
                    runtime_socket_connect,
                    runtime_socket_disconnect,
                    runtime_socket_state,
                    runtime_socket_emit,
                    // TDLib commands (native Telegram library)
                    tdlib_create_client,
                    tdlib_send,
                    tdlib_receive,
                    tdlib_destroy,
                    tdlib_is_available,
                    // Model commands (backend API proxy)
                    model_summarize,
                    model_generate,
                ]
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // Handle macOS Dock icon click (reopen event)
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => {
                    show_main_window(app_handle);
                }

                // Gracefully shut down TDLib before process exit to prevent
                // use-after-free crash in the blocking receive loop.
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                RunEvent::Exit => {
                    log::info!("[app] Exit event received, shutting down TDLib");
                    use crate::services::tdlib::TDLIB_MANAGER;
                    // Signal the TDLib worker to stop. The blocking receive() call
                    // has a 2-second internal timeout, so we must wait long enough
                    // for any in-flight call to finish before process teardown runs
                    // C++ destructors on TDLib's internal state.
                    TDLIB_MANAGER.signal_shutdown();
                    std::thread::sleep(std::time::Duration::from_millis(2500));
                    log::info!("[app] TDLib shutdown wait complete");
                    let _ = app_handle;
                }

                _ => {
                    let _ = app_handle;
                }
            }
        });
}
