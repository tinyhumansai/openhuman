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
        // V8 runtime commands
        runtime_discover_skills,
        runtime_list_skills,
        runtime_start_skill,
        runtime_stop_skill,
        runtime_get_skill_state,
        runtime_call_tool,
        runtime_all_tools,
        runtime_broadcast_event,
        // V8 runtime enable/disable + KV commands
        runtime_enable_skill,
        runtime_disable_skill,
        runtime_is_skill_enabled,
        runtime_get_skill_preferences,
        runtime_skill_kv_get,
        runtime_skill_kv_set,
        // V8 runtime JSON-RPC + data commands
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
    // Load .env file (silently ignore if missing — production won't have one)
    // Try current directory first, then parent (for when running from src-tauri)
    if dotenvy::dotenv().is_err() {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(parent) = cwd.parent() {
                let _ = dotenvy::from_path(parent.join(".env"));
            }
        }
    }

    // Initialize platform-appropriate logger
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("AlphaHuman"),
        );
        // Ensure vendored OpenSSL is initialized before any TLS usage
        openssl::init();
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = env_logger::try_init();
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

            // Initialize V8 Runtime Engine (desktop only - V8 not available on Android/iOS)
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

                // Initialize local model service (for skills to use)
                let model_dir = data_dir.join("models");
                services::llama::LLAMA_MANAGER.set_data_dir(model_dir);
                log::info!("[runtime] Local model service initialized");

                match runtime::v8_engine::RuntimeEngine::new(skills_data_dir) {
                    Ok(engine) => {
                        engine.set_app_handle(app.handle().clone());
                        let engine = std::sync::Arc::new(engine);

                        // Wire the SkillRegistry into the SocketManager for MCP
                        socket_mgr.set_registry(engine.registry());

                        app.manage(engine.clone());

                        // Start the cron scheduler
                        let cron = engine.cron_scheduler();
                        tauri::async_runtime::spawn(async move {
                            cron.start();
                        });

                        // Auto-start skills in background
                        let engine_clone = engine.clone();
                        tauri::async_runtime::spawn(async move {
                            engine_clone.auto_start_skills().await;
                        });

                        log::info!("[runtime] V8 runtime engine initialized");
                    }
                    Err(e) => {
                        log::error!("[runtime] Failed to initialize V8 runtime: {e}");
                    }
                }
            }

            #[cfg(target_os = "android")]
            {
                log::info!("[runtime] V8 runtime and local model disabled on Android");
            }

            #[cfg(target_os = "ios")]
            {
                log::info!("[runtime] V8 runtime and local model disabled on iOS");
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
                    // V8 runtime commands
                    runtime_discover_skills,
                    runtime_list_skills,
                    runtime_start_skill,
                    runtime_stop_skill,
                    runtime_get_skill_state,
                    runtime_call_tool,
                    runtime_all_tools,
                    runtime_broadcast_event,
                    // V8 runtime enable/disable + KV commands
                    runtime_enable_skill,
                    runtime_disable_skill,
                    runtime_is_skill_enabled,
                    runtime_get_skill_preferences,
                    runtime_skill_kv_get,
                    runtime_skill_kv_set,
                    // V8 runtime JSON-RPC + data commands
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
                    // Model commands (local LLM)
                    model_is_available,
                    model_get_status,
                    model_ensure_loaded,
                    model_generate,
                    model_summarize,
                    model_unload,
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
                    // V8 runtime commands
                    runtime_discover_skills,
                    runtime_list_skills,
                    runtime_start_skill,
                    runtime_stop_skill,
                    runtime_get_skill_state,
                    runtime_call_tool,
                    runtime_all_tools,
                    runtime_broadcast_event,
                    // V8 runtime enable/disable + KV commands
                    runtime_enable_skill,
                    runtime_disable_skill,
                    runtime_is_skill_enabled,
                    runtime_get_skill_preferences,
                    runtime_skill_kv_get,
                    runtime_skill_kv_set,
                    // V8 runtime JSON-RPC + data commands
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
                    // Model commands (local LLM)
                    model_is_available,
                    model_get_status,
                    model_ensure_loaded,
                    model_generate,
                    model_summarize,
                    model_unload,
                ]
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Handle macOS Dock icon click (reopen event)
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                show_main_window(app_handle);
            }

            // Suppress unused variable warnings on other platforms
            #[cfg(not(target_os = "macos"))]
            let _ = (app_handle, event);
        });
}
