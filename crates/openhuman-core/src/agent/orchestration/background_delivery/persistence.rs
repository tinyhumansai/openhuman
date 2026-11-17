//! Store a background delivery reply before publishing its terminal event.

use crate::core::runtime::DispatchCtx;

/// Persist a terminal reply, then announce it. A persistence failure returns
/// before `announce` runs, allowing the outer delivery loop to requeue the
/// completed background batch without publishing a phantom terminal event.
pub(super) fn persist_then_announce<P, A>(
    result: Result<String, String>,
    persist: P,
    announce: A,
) -> Result<String, String>
where
    P: FnOnce(&str, bool) -> Result<(), String>,
    A: FnOnce(&Result<String, String>),
{
    let (content, success) = match &result {
        Ok(text) => (text.trim().to_string(), true),
        Err(error) => (format!("Run failed: {error}"), false),
    };
    if !content.is_empty() {
        persist(&content, success)?;
    }
    announce(&result);
    result
}

/// Durably append a background-delivery reply before publishing its terminal
/// chat event. Callers must propagate failures: publishing `chat_done` or
/// `chat_error` without a stored row loses the result across reconnects.
pub(super) fn persist_delivery_reply(
    ctx: &DispatchCtx,
    workspace_dir: std::path::PathBuf,
    thread_id: &str,
    run_id: &str,
    content: String,
    success: bool,
) -> Result<(), String> {
    crate::threads::store::append_message(
        ctx,
        workspace_dir,
        thread_id,
        crate::threads::store::ConversationMessage {
            id: crate::threads::store::run_reply_message_id(run_id),
            content,
            message_type: "text".to_string(),
            extra_metadata: serde_json::json!({
                "scope": "background_delivery",
                "success": success,
                "requestId": run_id
            }),
            sender: "agent".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map(|_| ())
}
