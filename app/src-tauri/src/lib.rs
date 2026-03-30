//! OpenHuman Desktop Application
//!
//! This is the Rust backend for the cross-platform crypto community platform.
//! It provides deep link handling, core process RPC relay, window management,
//! and AI configuration helpers.

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host is desktop-only. Non-desktop targets are not supported.");

mod commands;
mod core_process;
mod core_rpc;
mod utils;

use commands::*;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tokio::time::{interval, Duration};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

/// Demo command - can be removed in production
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AIPreview {
    soul: AIPreviewSoul,
    tools: AIPreviewTools,
    metadata: AIPreviewMetadata,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AIPreviewSoul {
    raw: String,
    name: String,
    description: String,
    personality_preview: Vec<String>,
    safety_rules_preview: Vec<String>,
    loaded_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AIPreviewTools {
    raw: String,
    total_tools: usize,
    active_skills: usize,
    skills_preview: Vec<String>,
    loaded_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AIPreviewMetadata {
    loaded_at: i64,
    loading_duration: i64,
    has_fallbacks: bool,
    sources: AIPreviewSources,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AIPreviewSources {
    soul: String,
    tools: String,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn extract_section(raw: &str, heading: &str) -> String {
    let marker = format!("## {heading}");
    let Some(start) = raw.find(&marker) else {
        return String::new();
    };
    let body = &raw[start + marker.len()..];
    if let Some(next_idx) = body.find("\n## ") {
        body[..next_idx].trim().to_string()
    } else {
        body.trim().to_string()
    }
}

fn parse_soul_preview(raw: String, loaded_at: i64) -> AIPreviewSoul {
    let name = raw
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "OpenHuman".to_string());

    let description = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("AI assistant")
        .to_string();

    let personality_preview = extract_section(&raw, "Personality")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- **"))
        .filter_map(|line| {
            let mut parts = line.splitn(2, "**:");
            let trait_name = parts.next()?.trim();
            let detail = parts.next().unwrap_or("").trim();
            Some(format!("{trait_name}: {detail}"))
        })
        .take(3)
        .collect::<Vec<_>>();

    let safety_rules_preview = extract_section(&raw, "Safety Rules")
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let dot_idx = trimmed.find('.')?;
            let (prefix, rest) = trimmed.split_at(dot_idx);
            if prefix.chars().all(|c| c.is_ascii_digit()) {
                Some(rest.trim_start_matches('.').trim().to_string())
            } else {
                None
            }
        })
        .take(3)
        .collect::<Vec<_>>();

    AIPreviewSoul {
        raw,
        name,
        description,
        personality_preview,
        safety_rules_preview,
        loaded_at,
    }
}

fn parse_tools_preview(raw: String, loaded_at: i64) -> AIPreviewTools {
    let mut current_skill = "General".to_string();
    let mut skill_counts: HashMap<String, usize> = HashMap::new();
    let mut total_tools = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("### ") {
            if let Some(skill_title) = title.strip_suffix(" Tools") {
                current_skill = skill_title.trim().to_string();
                skill_counts.entry(current_skill.clone()).or_insert(0);
            }
            continue;
        }
        if trimmed.starts_with("#### ") {
            total_tools += 1;
            *skill_counts.entry(current_skill.clone()).or_insert(0) += 1;
        }
    }

    let mut skills = skill_counts.into_iter().collect::<Vec<_>>();
    let active_skills = skills.len();
    skills.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let skills_preview = skills
        .into_iter()
        .take(6)
        .map(|(name, count)| format!("{name} ({count})"))
        .collect::<Vec<_>>();

    AIPreviewTools {
        raw,
        total_tools,
        active_skills,
        skills_preview,
        loaded_at,
    }
}

fn resolve_ai_directory(app: &tauri::AppHandle) -> Option<(PathBuf, &'static str)> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        if let Some(ai_dir) = utils::dev_paths::bundled_openclaw_prompts_dir(&resource_dir) {
            return Some((ai_dir, "bundled"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(path) = utils::dev_paths::repo_ai_prompts_dir(&cwd) {
            return Some((path, "bundled"));
        }

        let fallback = cwd.join("ai");
        if fallback.is_dir() {
            return Some((fallback, "bundled"));
        }
    }

    None
}

fn build_ai_preview(app: &tauri::AppHandle) -> AIPreview {
    let started = now_ms();
    let loaded_at = now_ms();
    let mut errors = Vec::new();
    let mut soul_raw = String::new();
    let mut tools_raw = String::new();
    let mut source = "bundled".to_string();

    if let Some((ai_dir, resolved_source)) = resolve_ai_directory(app) {
        source = resolved_source.to_string();
        let soul_path = ai_dir.join("SOUL.md");
        let tools_path = ai_dir.join("TOOLS.md");
        soul_raw = std::fs::read_to_string(&soul_path).unwrap_or_else(|e| {
            errors.push(format!("Failed to read SOUL.md: {e}"));
            String::new()
        });
        tools_raw = std::fs::read_to_string(&tools_path).unwrap_or_else(|e| {
            errors.push(format!("Failed to read TOOLS.md: {e}"));
            String::new()
        });
    } else {
        errors.push("AI config directory not found".to_string());
    }

    let soul = parse_soul_preview(soul_raw, loaded_at);
    let tools = parse_tools_preview(tools_raw, loaded_at);
    let done = now_ms();

    AIPreview {
        soul,
        tools,
        metadata: AIPreviewMetadata {
            loaded_at: done,
            loading_duration: done - started,
            has_fallbacks: false,
            sources: AIPreviewSources {
                soul: source.clone(),
                tools: source,
            },
            errors,
        },
    }
}

#[tauri::command]
async fn ai_get_config(app: tauri::AppHandle) -> Result<AIPreview, String> {
    Ok(build_ai_preview(&app))
}

#[tauri::command]
async fn ai_refresh_config(app: tauri::AppHandle) -> Result<AIPreview, String> {
    Ok(build_ai_preview(&app))
}

/// Write AI configuration files to `src/openhuman/agent/prompts` in the repo (dev resolution from cwd).
#[tauri::command]
async fn write_ai_config_file(filename: String, content: String) -> Result<bool, String> {
    use std::env;

    // Determine runtime working directory
    let current_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;

    // Ensure filename is safe (only allow .md files)
    if !filename.ends_with(".md") {
        return Err("Only .md files are allowed".to_string());
    }

    // Prevent path traversal by checking for dangerous characters
    if filename.contains("..") || filename.contains("/") || filename.contains("\\") {
        return Err("Invalid filename: path traversal not allowed".to_string());
    }

    let ai_dir = utils::dev_paths::repo_ai_prompts_dir(&current_dir).unwrap_or_else(|| {
        current_dir
            .join("src")
            .join("openhuman")
            .join("agent")
            .join("prompts")
    });
    let file_path = ai_dir.join(&filename);

    // Ensure ai directory exists
    std::fs::create_dir_all(&ai_dir).map_err(|e| format!("Failed to create ai directory: {e}"))?;

    // Write the file
    std::fs::write(&file_path, content)
        .map_err(|e| format!("Failed to write file {}: {e}", filename))?;

    Ok(true)
}

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

/// Poll core RPC health and bridge updates to frontend Tauri events.
async fn watch_daemon_health_rpc(app_handle: AppHandle) {
    let mut interval = interval(Duration::from_secs(2));
    let mut last_snapshot: Option<serde_json::Value> = None;
    let mut had_error = false;

    log::info!("[openhuman] Watching daemon health via core RPC (openhuman.health_snapshot)");

    loop {
        interval.tick().await;

        match crate::core_rpc::call::<serde_json::Value>("openhuman.health_snapshot", json!({}))
            .await
        {
            Ok(raw_payload) => {
                // RpcOutcome may be wrapped as {"result": {...}, "logs": [...]}; normalize to snapshot.
                let snapshot = raw_payload.get("result").cloned().unwrap_or(raw_payload);

                if last_snapshot.as_ref() != Some(&snapshot) {
                    if let Err(e) = app_handle.emit("openhuman:health", &snapshot) {
                        log::error!("[openhuman] Failed to emit health event from RPC: {e}");
                    } else {
                        last_snapshot = Some(snapshot);
                    }
                }

                if had_error {
                    had_error = false;
                    log::info!("[openhuman] Health RPC polling recovered");
                }
            }
            Err(e) => {
                if !had_error {
                    had_error = true;
                    log::debug!("[openhuman] Health RPC not ready yet: {e}");
                }
            }
        }
    }
}

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

    // Initialize logger
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
            // Register deep link handlers (Windows/Linux)
            #[cfg(any(windows, target_os = "linux"))]
            {
                app.deep_link().register_all()?;
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

            // Bridge daemon health via core RPC and ensure core background service.
            {
                let app_handle_for_watcher = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    watch_daemon_health_rpc(app_handle_for_watcher).await;
                });
                tauri::async_runtime::spawn(async move {
                    match commands::core_relay::ensure_service_managed_core_running().await {
                        Ok(()) => {
                            log::info!("[openhuman] Core background service ensured via core RPC");
                        }
                        Err(e) => {
                            log::error!(
                                "[openhuman] Failed to ensure core background service: {e}"
                            );
                        }
                    }
                });
            }

            // Start/ensure standalone core process for business logic RPC.
            {
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
            }

            if daemon_mode {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            Ok(())
        })
        // Register all commands (desktop build lists handlers explicitly below).
        .invoke_handler({
            #[cfg(desktop)]
            {
                tauri::generate_handler![
                    greet,
                    // AI config file writing
                    write_ai_config_file,
                    ai_get_config,
                    ai_refresh_config,
                    core_rpc_relay,
                    core_rpc_url,
                    show_window,
                    hide_window,
                    toggle_window,
                    is_window_visible,
                    minimize_window,
                    maximize_window,
                    close_window,
                    set_window_title,
                    // OpenHuman local host commands (core RPC uses core_rpc_relay)
                    openhuman_get_daemon_host_config,
                    openhuman_set_daemon_host_config,
                    openhuman_service_install,
                    openhuman_service_start,
                    openhuman_service_stop,
                    openhuman_service_status,
                    openhuman_service_uninstall,
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
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                }

                // Gracefully shut down background services before process exit.
                RunEvent::Exit => {
                    log::info!("[app] Exit event received, shutting down");

                    let _ = app_handle;
                }

                _ => {
                    let _ = app_handle;
                }
            }
        });
}

pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    let core_bin = crate::core_process::default_core_bin()
        .ok_or_else(|| anyhow::anyhow!("openhuman core binary not found"))?;
    let status = std::process::Command::new(core_bin)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to execute core binary: {e}"))?;
    if !status.success() {
        anyhow::bail!("core binary exited with status {status}");
    }
    Ok(())
}
