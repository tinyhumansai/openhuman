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
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tokio::time::{interval, Duration};

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

async fn watch_daemon_health_rpc(app_handle: AppHandle) {
    let mut ticker = interval(Duration::from_secs(2));
    let mut last_snapshot: Option<serde_json::Value> = None;

    loop {
        ticker.tick().await;

        if let Ok(raw_payload) =
            crate::core_rpc::call::<serde_json::Value>("openhuman.health_snapshot", json!({})).await
        {
            let snapshot = raw_payload.get("result").cloned().unwrap_or(raw_payload);
            if last_snapshot.as_ref() != Some(&snapshot) {
                if let Err(e) = app_handle.emit("openhuman:health", &snapshot) {
                    log::error!("[openhuman] Failed to emit health event from RPC: {e}");
                } else {
                    last_snapshot = Some(snapshot);
                }
            }
        }
    }
}

pub fn run() {
    let daemon_mode = is_daemon_mode();

    let default_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "info,reqwest=warn,hyper=warn,h2=warn".to_string());
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

            {
                let app_handle_for_watcher = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    watch_daemon_health_rpc(app_handle_for_watcher).await;
                });
            }

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
        .invoke_handler(tauri::generate_handler![
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
            openhuman_get_daemon_host_config,
            openhuman_set_daemon_host_config,
        ])
        .build({
            let mut context = tauri::generate_context!();
            if daemon_mode {
                context.config_mut().app.windows.clear();
            }
            context
        })
        .expect("error while building tauri application")
        .run(move |app_handle, event| match event {
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
        .ok_or_else(|| "openhuman core binary not found".to_string())?;
    let status = std::process::Command::new(core_bin)
        .args(args)
        .status()
        .map_err(|e| format!("failed to execute core binary: {e}"))?;
    if !status.success() {
        return Err(format!("core binary exited with status {status}"));
    }
    Ok(())
}
