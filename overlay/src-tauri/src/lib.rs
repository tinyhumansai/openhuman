mod log_bridge;

use log_bridge::{LogBuffer, LogEntry, TauriLogLayer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Tauri state holding the log ring buffer and click-through toggle.
struct OverlayState {
    log_buffer: Arc<LogBuffer>,
    click_through: Arc<AtomicBool>,
}

// ── Tauri commands ──────────────────────────────────────────────────────────

/// Return all buffered log entries (for initial load / reconnect).
#[tauri::command]
fn get_log_history(state: tauri::State<'_, OverlayState>) -> Vec<LogEntry> {
    state.log_buffer.snapshot()
}

/// Toggle click-through mode. When enabled, mouse events pass through
/// the overlay to the window underneath.
#[tauri::command]
fn set_click_through(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, OverlayState>,
    enabled: bool,
) -> Result<(), String> {
    state.click_through.store(enabled, Ordering::Relaxed);
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|e| e.to_string())?;
    log::debug!("[overlay] click-through set to {}", enabled);
    Ok(())
}

/// Forward an RPC call to openhuman_core's dispatch in-process.
/// Uses the same invoke_method path as the HTTP JSON-RPC server.
#[tauri::command]
async fn core_rpc(method: String, params: serde_json::Value) -> Result<serde_json::Value, String> {
    log::debug!("[overlay] core_rpc: method={}", method);
    let state = openhuman_core::core::jsonrpc::default_state();
    openhuman_core::core::jsonrpc::invoke_method(state, &method, params).await
}

/// Insert text into the currently focused field in the previously active app.
#[tauri::command]
fn insert_text_into_focused_field(text: String) -> Result<(), String> {
    log::debug!(
        "[overlay] insert_text_into_focused_field len={}",
        text.chars().count()
    );
    openhuman_core::openhuman::accessibility::apply_text_to_focused_field(&text)
}

// ── App entry ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Shared state
    let log_buffer = Arc::new(LogBuffer::new(5000));
    let click_through = Arc::new(AtomicBool::new(false));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(OverlayState {
            log_buffer: log_buffer.clone(),
            click_through: click_through.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            get_log_history,
            set_click_through,
            core_rpc,
            insert_text_into_focused_field,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // ── Tracing subscriber with Tauri bridge layer ──────────────
            let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new(
                    "debug,hyper=info,reqwest=info,tungstenite=info,tokio_tungstenite=info",
                )
            });

            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_ansi(true);

            let tauri_layer = TauriLogLayer::new(app_handle.clone(), log_buffer.clone());

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(tauri_layer)
                .init();

            // Bridge `log` crate macros into tracing
            tracing_log::LogTracer::init().ok();

            log::info!("[overlay] overlay process started, tracing bridge active");

            // ── Start openhuman_core JSON-RPC server in-process ─────────
            // Use port 7799 to avoid conflicts with a standalone core on 7788.
            // Override with OPENHUMAN_CORE_PORT env var.
            tauri::async_runtime::spawn(async move {
                let port = std::env::var("OPENHUMAN_CORE_PORT")
                    .ok()
                    .and_then(|p| p.parse::<u16>().ok())
                    .unwrap_or(7799);
                log::info!("[overlay] starting openhuman_core server on 127.0.0.1:{}...", port);
                match openhuman_core::core::jsonrpc::run_server(None, Some(port), true).await {
                    Ok(()) => log::info!("[overlay] core server shut down cleanly"),
                    Err(e) => log::error!("[overlay] core server error: {}", e),
                }
            });

            // ── macOS: floating panel + visible on all workspaces ───────
            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("overlay") {
                    window.set_always_on_top(true).ok();
                    window.set_visible_on_all_workspaces(true).ok();
                    log::debug!("[overlay] macOS: set always-on-top + visible-on-all-workspaces");
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running overlay");
}
