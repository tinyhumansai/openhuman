//! Conversation send, cancel and session loading.
use super::*;
use openhuman_rpc::embed::chat_surface::WebChannelEvent;

/// Queue a chat turn on the current thread. Fire-and-forget: the reply streams
/// back over the web-channel bus and is folded in by the reducer.
pub(super) fn send_message(
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
    queue_mode: &str,
) {
    if ui.thread_id.is_empty() {
        state.push_system("Create or select a conversation first; your draft is preserved.");
        return;
    }
    if ui.auth_pending {
        state.push_system("Authentication is still in progress; your draft is preserved.");
        return;
    }
    let Some(message) = ui.composer.take_for_send() else {
        return;
    };
    ui.scroll_from_bottom = 0;
    state.begin_user_turn(&message);
    log::info!(
        "[tui] send message len={} thread={}",
        message.len(),
        ui.thread_id
    );

    let rt = runtime.clone();
    let cid = client_id.to_string();
    let tid = ui.thread_id.clone();
    let mode = queue_mode.to_string();
    let model_override = ui.model_override.clone();
    tokio::spawn(async move {
        let params = json!({
            "client_id": cid,
            "thread_id": tid,
            "message": message,
            "source": "type",
            "queue_mode": mode,
            "model_override": model_override,
        });
        if let Err(e) = rt.invoke("openhuman.channel_web_chat", params).await {
            log::error!("[tui] openhuman.channel_web_chat failed: {e}");
            // Surface the failure in-transcript via a synthetic chat_error so
            // the reducer clears the streaming state and shows the reason.
            web_chat::publish_web_channel_event(WebChannelEvent::ChatError {
                context: web_chat::WebEventContext {
                    client_id: cid,
                    thread_id: tid,
                    ..Default::default()
                },
                message: Some(format!("Failed to send: {e}")),
                error_type: Some("transport".to_string()),
                error_source: None,
                error_retryable: None,
                error_retry_after_ms: None,
                error_provider: None,
                error_fallback_available: None,
                copy_key: None,
                copy_params: None,
                guardrail: None,
            });
        }
    });
}

/// Cancel the in-flight turn on the current thread. The core emits a
/// `chat_error` ("Cancelled") which the reducer renders.
pub(super) fn cancel_turn(
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    thread_id: &str,
    state: &TranscriptState,
) {
    if !state.is_streaming() {
        return;
    }
    log::info!("[tui] cancel turn thread={thread_id}");
    let rt = runtime.clone();
    let cid = client_id.to_string();
    let tid = thread_id.to_string();
    tokio::spawn(async move {
        // Omit `request_id` → stop whatever is running on the thread.
        let params = json!({ "client_id": cid, "thread_id": tid });
        if let Err(e) = rt.invoke("openhuman.channel_web_cancel", params).await {
            log::error!("[tui] openhuman.channel_web_cancel failed: {e}");
        }
    });
}

/// Create a fresh thread and switch the UI to it. Awaited inline (fast, local
/// SQLite write) so `ui.thread_id` can be updated with the result.
pub(super) async fn new_thread(
    runtime: &Arc<CoreRuntime>,
    state: &mut TranscriptState,
    ui: &mut UiState,
) {
    log::info!("[tui] creating new thread");
    match runtime
        .invoke("openhuman.threads_create_new", json!({}))
        .await
        .ok()
        .and_then(|v| crate::runner::extract_thread_id(&v))
    {
        Some(new_id) => {
            ui.drafts
                .insert(ui.thread_id.clone(), ui.composer.text().to_string());
            ui.composer.clear();
            ui.viewport = Default::default();
            let client_id = state.client_id().to_string();
            *state = TranscriptState::new(client_id);
            ui.thread_id = new_id.clone();
            state.set_thread(new_id.clone());
            ui.scroll_from_bottom = 0;
            state.push_system(format!("Started a new thread · {new_id}"));
            log::info!("[tui] switched to new thread {new_id}");
        }
        None => {
            state.push_system("Could not create a new thread (see logs).".to_string());
            log::error!("[tui] threads.create_new returned no thread id");
        }
    }
}
pub(super) async fn switch_thread(
    runtime: &Arc<CoreRuntime>,
    state: &mut TranscriptState,
    ui: &mut UiState,
    thread_id: String,
) {
    match runtime
        .invoke(
            "openhuman.threads_transcript_get",
            json!({"thread_id":thread_id,"limit":500}),
        )
        .await
    {
        Ok(value) => {
            ui.drafts
                .insert(ui.thread_id.clone(), ui.composer.text().to_string());
            let client_id = state.client_id().to_string();
            *state = TranscriptState::new(client_id);
            state.set_thread(thread_id.clone());
            state.load_transcript(&value);
            ui.composer
                .set_text(ui.drafts.get(&thread_id).cloned().unwrap_or_default());
            ui.thread_id = thread_id;
            ui.viewport = Default::default();
            ui.scroll_from_bottom = 0;
            ui.overlay = None;
        }
        Err(error) => {
            if let Some(overlay) = &mut ui.overlay {
                overlay.status = format!("History unavailable: {error}");
            }
        }
    }
}

pub(super) async fn load_transcript(
    runtime: &Arc<CoreRuntime>,
    state: &mut TranscriptState,
    thread_id: &str,
) {
    match runtime
        .invoke(
            "openhuman.threads_transcript_get",
            json!({"thread_id": thread_id, "limit": 500}),
        )
        .await
    {
        Ok(value) => state.load_transcript(&value),
        Err(error) => log::warn!("[tui] could not load transcript thread={thread_id}: {error}"),
    }
}

pub(super) async fn refresh_agent_paths(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    if let Ok(value) = runtime
        .invoke("openhuman.config_get_agent_paths", json!({}))
        .await
    {
        let paths = crate::cockpit::unwrap_rpc(&value);
        ui.action_dir = paths
            .get("action_dir")
            .or_else(|| paths.get("actionDir"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
}
