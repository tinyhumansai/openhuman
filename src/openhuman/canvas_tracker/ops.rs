use chrono::Utc;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::auth::{clear_canvas_token, get_canvas_token, store_canvas_token};
use super::store::CanvasTrackerStore;
use super::types::{CanvasTask, CanvasTrackerSettings, LocalStatus, SyncSummary};

pub async fn get_settings(config: &Config) -> Result<RpcOutcome<CanvasTrackerSettings>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let mut settings = store.get_settings().map_err(|e| e.to_string())?;
    settings.token_set = get_canvas_token(config)?.is_some();
    Ok(RpcOutcome::single_log(
        settings,
        "canvas tracker settings loaded",
    ))
}

pub async fn update_settings(
    config: &Config,
    mut settings: CanvasTrackerSettings,
    token: Option<String>,
    clear_token: bool,
) -> Result<RpcOutcome<CanvasTrackerSettings>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    settings.enforce_approved_allowlist();

    if clear_token {
        clear_canvas_token(config).await?;
    }
    if let Some(token) = token {
        store_canvas_token(config, &token).await?;
    }

    settings.token_set = get_canvas_token(config)?.is_some();
    store.save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        settings,
        "canvas tracker settings saved",
    ))
}

pub async fn sync_now(config: &Config) -> Result<RpcOutcome<SyncSummary>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let settings = store.get_settings().map_err(|e| e.to_string())?;
    let token =
        get_canvas_token(config)?.ok_or_else(|| "canvas token is not configured".to_string())?;
    let summary = super::sync::sync_once(store, &settings, &token, Utc::now()).await?;
    Ok(RpcOutcome::single_log(
        summary,
        "canvas tracker sync complete",
    ))
}

pub async fn list_tasks(config: &Config) -> Result<RpcOutcome<Vec<CanvasTask>>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        store.list_tasks().map_err(|e| e.to_string())?,
        "canvas tracker tasks loaded",
    ))
}

pub async fn update_local_status(
    config: &Config,
    course_id: &str,
    assignment_id: &str,
    status: LocalStatus,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    store
        .update_local_status(course_id, assignment_id, status)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "updated": true }),
        "canvas tracker local status updated",
    ))
}

pub async fn list_reminders(
    config: &Config,
) -> Result<RpcOutcome<Vec<super::types::ReminderRecommendation>>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let reminders = store
        .list_tasks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|task| task.reminders_needed)
        .collect();
    Ok(RpcOutcome::single_log(
        reminders,
        "canvas tracker reminders loaded",
    ))
}
