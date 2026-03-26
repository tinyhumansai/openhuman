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
pub mod alphahuman;
mod auth;
mod commands;
mod core_process;
mod core_rpc;
pub mod core_server;
pub mod memory;
mod models;
mod runtime;
mod services;
mod unified_skills;
mod utils;

use ai::*;
use commands::chat::ChatState;
use commands::unified_skills::{
    unified_execute_skill, unified_generate_skill, unified_list_skills, unified_self_evolve_skill,
};
use commands::*;
use services::socket_service::SOCKET_SERVICE;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tokio::{
    fs,
    time::{interval, Duration},
};

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

/// Write AI configuration files to the project's ./ai/ directory
#[tauri::command]
async fn write_ai_config_file(filename: String, content: String) -> Result<bool, String> {
    use std::env;

    // Get the project root directory (parent of src-tauri)
    let current_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;

    // Go up one level to get project root (since we're in src-tauri/)
    let project_root = current_dir
        .parent()
        .ok_or("Failed to get project root directory")?;

    // Ensure filename is safe (only allow .md files)
    if !filename.ends_with(".md") {
        return Err("Only .md files are allowed".to_string());
    }

    // Prevent path traversal by checking for dangerous characters
    if filename.contains("..") || filename.contains("/") || filename.contains("\\") {
        return Err("Invalid filename: path traversal not allowed".to_string());
    }

    // Create path to ai/ directory in project root
    let ai_dir = project_root.join("ai");
    let file_path = ai_dir.join(&filename);

    // Ensure ai directory exists
    std::fs::create_dir_all(&ai_dir).map_err(|e| format!("Failed to create ai directory: {e}"))?;

    // Write the file
    std::fs::write(&file_path, content)
        .map_err(|e| format!("Failed to write file {}: {e}", filename))?;

    Ok(true)
}

// Macro to define common handlers shared across all platforms
macro_rules! common_handlers {
    () => {
        // Demo
        greet,
        // AI config file writing
        write_ai_config_file,
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
        runtime_get_tool_schemas,
        runtime_execute_tool,
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

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

fn daemon_foreground_requested() -> bool {
    matches!(
        std::env::var("ALPHAHUMAN_DAEMON_FOREGROUND")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
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

/// Watch daemon health file and bridge changes to frontend Tauri events
async fn watch_daemon_health_file(app_handle: AppHandle, data_dir: PathBuf) {
    let state_file = data_dir.join("daemon_state.json");
    let mut interval = interval(Duration::from_secs(2));
    let mut last_modified: Option<std::time::SystemTime> = None;

    log::info!(
        "[alphahuman] Watching daemon health file: {}",
        state_file.display()
    );

    loop {
        interval.tick().await;

        // Check if file exists and was modified
        if let Ok(metadata) = fs::metadata(&state_file).await {
            if let Ok(modified) = metadata.modified() {
                if last_modified.map_or(true, |last| modified > last) {
                    last_modified = Some(modified);

                    // Read and parse health data
                    if let Ok(content) = fs::read_to_string(&state_file).await {
                        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&content)
                        {
                            log::debug!(
                                "[alphahuman] Broadcasting health event from file: {:?}",
                                json_value
                            );

                            // Emit Tauri event to frontend (same as internal daemon)
                            if let Err(e) = app_handle.emit("alphahuman:health", &json_value) {
                                log::error!(
                                    "[alphahuman] Failed to emit health event from file: {}",
                                    e
                                );
                            } else {
                                log::debug!(
                                    "[alphahuman] Health event emitted successfully from file"
                                );
                            }
                        } else {
                            log::debug!(
                                "[alphahuman] Failed to parse health file as JSON: {}",
                                state_file.display()
                            );
                        }
                    } else {
                        log::debug!(
                            "[alphahuman] Failed to read health file: {}",
                            state_file.display()
                        );
                    }
                }
            }
        } else {
            // File doesn't exist yet - external daemon may not be writing yet
            log::debug!(
                "[alphahuman] Health file not found yet: {}",
                state_file.display()
            );
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(err) = rustls::crypto::ring::default_provider().install_default() {
        log::warn!(
            "[app] rustls crypto provider not installed (already set?): {:?}",
            err
        );
    } else {
        log::info!("[app] rustls crypto provider installed (ring)");
    }

    let daemon_mode = is_daemon_mode();

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

                // Extract tag from message (e.g. "[socket-mgr]", "[skill:x]")
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
                    if t_lower.contains("socket") {
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
                Some(vec!["--daemon"]),
            ))
            .plugin(tauri_plugin_notification::init());
    }

    builder
        // Setup
        .setup(move |app| {
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
                if daemon_mode {
                    setup_tray(app.handle())?;
                }
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

            // Start the alphahuman daemon supervisor (desktop only)
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".alphahuman")
                    });
                let daemon_config = alphahuman::config::DaemonConfig::from_app_data_dir(&data_dir);
                let cancel = tokio_util::sync::CancellationToken::new();
                let daemon_handle = alphahuman::daemon::DaemonHandle {
                    cancel: cancel.clone(),
                };
                app.manage(daemon_handle);

                // Determine daemon mode: internal supervisor vs external platform service
                let use_internal_daemon = daemon_mode
                    || daemon_foreground_requested()
                    || cfg!(debug_assertions)  // Always use internal supervisor in debug builds
                    || std::env::var("ALPHAHUMAN_DAEMON_INTERNAL").unwrap_or("false".to_string()) == "true";  // Cross-platform override via env var

                if use_internal_daemon {
                    // Run internal daemon supervisor with health event emission
                    // This path is taken when:
                    // - Daemon mode enabled, OR
                    // - Foreground daemon requested, OR
                    // - Debug build (for easier development), OR
                    // - ALPHAHUMAN_DAEMON_INTERNAL=true env var (any platform)
                    log::info!("[alphahuman] Using internal daemon supervisor (ALPHAHUMAN_DAEMON_INTERNAL=true or debug build)");
                    let app_handle_for_daemon = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        log::info!("[alphahuman] Starting daemon supervisor with health monitoring");
                        if let Err(e) = alphahuman::daemon::run(
                            daemon_config,
                            app_handle_for_daemon,
                            cancel,
                        )
                        .await
                        {
                            log::error!("[alphahuman] Daemon supervisor error: {e}");
                        }
                    });
                } else {
                    // Start external platform-specific service for background daemon
                    // This path is taken on all platforms when ALPHAHUMAN_DAEMON_INTERNAL=false/unset
                    // and not in daemon mode, foreground mode, or debug build
                    log::info!("[alphahuman] Using external daemon service (ALPHAHUMAN_DAEMON_INTERNAL=false/unset)");

                    // Setup file watching to bridge external daemon health events to frontend
                    let app_handle_for_watcher = app.handle().clone();
                    let data_dir_clone = data_dir.clone();
                    tauri::async_runtime::spawn(async move {
                        watch_daemon_health_file(app_handle_for_watcher, data_dir_clone).await;
                    });

                    // Start the external platform service
                    tauri::async_runtime::spawn(async move {
                        match alphahuman::config::Config::load_or_init().await {
                            Ok(config) => {
                                match alphahuman::service::install(&config) {
                                    Ok(status) => log::info!("[alphahuman] External daemon service installed: {:?}", status),
                                    Err(e) => log::error!("[alphahuman] Failed to install external daemon service: {e}"),
                                }
                                match alphahuman::service::start(&config) {
                                    Ok(status) => log::info!("[alphahuman] External daemon service started: {:?}", status),
                                    Err(e) => log::error!("[alphahuman] Failed to start external daemon service: {e}"),
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "[alphahuman] Failed to load config for external service: {e}"
                                );
                            }
                        }
                    });
                }
            }

            // Start/ensure standalone core process for business logic RPC.
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let core_handle = core_process::CoreProcessHandle::new(core_process::default_core_port());
                std::env::set_var("ALPHAHUMAN_CORE_RPC_URL", core_handle.rpc_url());
                app.manage(core_handle.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = core_handle.ensure_running().await {
                        log::error!("[core] failed to start core process: {err}");
                    } else {
                        log::info!("[core] core process ready");
                    }
                });
            }

            if daemon_mode {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Initialize TinyHumans memory state (empty until the frontend provides the JWT)
            app.manage(commands::memory::MemoryState(std::sync::Mutex::new(None)));
            log::info!("[memory] Memory state registered — awaiting JWT from frontend");

            // Initialize ChatState for managing in-flight conversation requests
            let chat_state = std::sync::Arc::new(ChatState::new());
            app.manage(chat_state);
            log::info!("[chat] ChatState registered");

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
                    // AI config file writing
                    write_ai_config_file,
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
                    runtime_get_tool_schemas,
                    runtime_execute_tool,
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
                    // Telegram commands removed (unified system eliminated as per user request)
                    // Model commands (backend API proxy)
                    model_summarize,
                    model_generate,
                    // Alphahuman commands
                    alphahuman_health_snapshot,
                    alphahuman_security_policy_info,
                    alphahuman_encrypt_secret,
                    alphahuman_decrypt_secret,
                    alphahuman_get_config,
                    alphahuman_update_model_settings,
                    alphahuman_update_memory_settings,
                    alphahuman_update_gateway_settings,
                    alphahuman_update_tunnel_settings,
                    alphahuman_update_runtime_settings,
                    alphahuman_update_browser_settings,
                    alphahuman_get_runtime_flags,
                    alphahuman_set_browser_allow_all,
                    alphahuman_agent_chat,
                    alphahuman_doctor_report,
                    alphahuman_doctor_models,
                    alphahuman_list_integrations,
                    alphahuman_get_integration_info,
                    alphahuman_models_refresh,
                    alphahuman_migrate_openclaw,
                    alphahuman_hardware_discover,
                    alphahuman_hardware_introspect,
                    alphahuman_service_install,
                    alphahuman_service_start,
                    alphahuman_service_stop,
                    alphahuman_service_status,
                    alphahuman_service_uninstall,
                    alphahuman_agent_server_status,
                    // Unified skill registry commands
                    unified_list_skills,
                    unified_execute_skill,
                    unified_generate_skill,
                    unified_self_evolve_skill,
                    // Memory commands (TinyHumans Neocortex)
                    init_memory_client,
                    memory_query,
                    recall_memory,
                    memory_list_documents,
                    memory_list_namespaces,
                    memory_delete_document,
                    memory_query_namespace,
                    memory_recall_namespace,
                    // Chat commands (agentic conversation loop)
                    chat_send,
                    chat_cancel,
                ]
            }
            #[cfg(not(desktop))]
            {
                tauri::generate_handler![
                    // Common handlers (expanded from common_handlers! macro)
                    greet,
                    // AI config file writing
                    write_ai_config_file,
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
                    runtime_get_tool_schemas,
                    runtime_execute_tool,
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
                    // Telegram commands removed (unified system eliminated as per user request)
                    // Model commands (backend API proxy)
                    model_summarize,
                    model_generate,
                    // Alphahuman commands
                    alphahuman_health_snapshot,
                    alphahuman_security_policy_info,
                    alphahuman_encrypt_secret,
                    alphahuman_decrypt_secret,
                    alphahuman_get_config,
                    alphahuman_update_model_settings,
                    alphahuman_update_memory_settings,
                    alphahuman_update_gateway_settings,
                    alphahuman_update_tunnel_settings,
                    alphahuman_update_runtime_settings,
                    alphahuman_update_browser_settings,
                    alphahuman_get_runtime_flags,
                    alphahuman_set_browser_allow_all,
                    alphahuman_agent_chat,
                    alphahuman_doctor_report,
                    alphahuman_doctor_models,
                    alphahuman_list_integrations,
                    alphahuman_get_integration_info,
                    alphahuman_models_refresh,
                    alphahuman_migrate_openclaw,
                    alphahuman_hardware_discover,
                    alphahuman_hardware_introspect,
                    alphahuman_service_install,
                    alphahuman_service_start,
                    alphahuman_service_stop,
                    alphahuman_service_status,
                    alphahuman_service_uninstall,
                    alphahuman_agent_server_status,
                    // Unified skill registry commands (mobile stubs)
                    unified_list_skills,
                    unified_execute_skill,
                    unified_generate_skill,
                    unified_self_evolve_skill,
                    // Memory commands (TinyHumans Neocortex)
                    init_memory_client,
                    memory_query,
                    recall_memory,
                    memory_list_documents,
                    memory_list_namespaces,
                    memory_delete_document,
                    memory_query_namespace,
                    memory_recall_namespace,
                    // Chat commands (agentic conversation loop)
                    chat_send,
                    chat_cancel,
                ]
            }
        })
        .build({
            let mut context = tauri::generate_context!();
            if daemon_mode {
                context.config_mut().app.windows.clear();
            }
            context
        })
        .expect("error while building tauri application")
        .run(move |app_handle, event| {
            match event {
                // Handle macOS Dock icon click (reopen event)
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => {
                    if !daemon_mode {
                        show_main_window(app_handle);
                    }
                }

                // Gracefully shut down TDLib before process exit to prevent
                // use-after-free crash in the blocking receive loop.
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                RunEvent::Exit => {
                    log::info!("[app] Exit event received, shutting down");

                    // Cancel the alphahuman daemon supervisor
                    if let Some(daemon) = app_handle.try_state::<alphahuman::daemon::DaemonHandle>() {
                        daemon.cancel.cancel();
                        log::info!("[alphahuman] Daemon shutdown signalled");
                    }

                    if let Some(core) = app_handle.try_state::<core_process::CoreProcessHandle>() {
                        let core_handle: core_process::CoreProcessHandle = (*core).clone();
                        tauri::async_runtime::spawn(async move {
                            core_handle.shutdown().await;
                        });
                    }

                    let _ = app_handle;
                }

                _ => {
                    let _ = app_handle;
                }
            }
        });
}

pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    core_server::run_from_cli_args(args)
}
