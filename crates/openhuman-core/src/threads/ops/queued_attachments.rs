//! Persistence helpers for admitted queued web-chat user messages.

use super::support::message_to_record;
use crate::core::runtime::DispatchCtx;
use crate::threads::store as conversations;
use crate::threads::{AppendConversationMessageRequest, ConversationMessageRecord};

/// Persist an admitted queued web user message under its stable request ID.
///
/// This is called from the trusted turn runner with the same scoped dispatch
/// context used to admit and resolve the queued attachments. The row is
/// appended once in turn order; retries return it only when its text and
/// opaque attachment references match exactly.
pub(crate) async fn persist_queued_user_message(
    ctx: &DispatchCtx,
    thread_id: &str,
    request_id: &str,
    text: &str,
    opaque_parts: &[tinychannels_bus::ContentPart],
) -> Result<ConversationMessageRecord, String> {
    if thread_id.trim().is_empty() || request_id.trim().is_empty() {
        return Err("queued message thread and request IDs must be non-empty".into());
    }
    if !opaque_parts.is_empty() {
        crate::threads::attachments::validate_opaque_parts(opaque_parts)?;
    }
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("load queued message configuration: {error}"))?;
    persist_queued_user_message_with_config(ctx, thread_id, request_id, text, opaque_parts, &config)
        .await
}

async fn persist_queued_user_message_with_config(
    ctx: &DispatchCtx,
    thread_id: &str,
    request_id: &str,
    text: &str,
    opaque_parts: &[tinychannels_bus::ContentPart],
    config: &crate::config::Config,
) -> Result<ConversationMessageRecord, String> {
    let row_id = format!("user:{request_id}");
    let existing = conversations::get_messages(ctx, config.workspace_dir.clone(), thread_id)?
        .into_iter()
        .find(|message| message.id == row_id);
    if let Some(existing) = existing {
        let stored_parts = crate::threads::attachments::public_refs(&existing.extra_metadata);
        if existing.sender != "user" || existing.content != text || stored_parts != opaque_parts {
            return Err("queued message request ID conflicts with an existing row".into());
        }
        if !opaque_parts.is_empty() {
            crate::threads::attachments::consume_staged_parts(
                ctx,
                thread_id,
                opaque_parts,
                &row_id,
            )
            .await?;
        }
        return Ok(message_to_record(existing));
    }

    let origin = ctx.turn_origin_or_scoped();
    let request = AppendConversationMessageRequest {
        thread_id: thread_id.to_string(),
        message: ConversationMessageRecord {
            id: row_id.clone(),
            content: text.to_string(),
            attachments: Vec::new(),
            message_type: "text".into(),
            extra_metadata: serde_json::json!({}),
            sender: "user".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
        attachments: opaque_parts.to_vec(),
    };
    let outcome = super::crud::message_append_with_config(
        ctx,
        request,
        config,
        origin.as_ref(),
        &config.workspace_dir,
    )
    .await
    .map_err(|error| error.to_string())?;
    let persisted = outcome
        .value
        .data
        .ok_or_else(|| "queued message append returned no row".to_string())?;
    if persisted.id != row_id
        || persisted.sender != "user"
        || persisted.content != text
        || persisted.attachments != opaque_parts
    {
        if !opaque_parts.is_empty() {
            let _ = crate::threads::attachments::discard_staged_parts(ctx, thread_id, opaque_parts)
                .await;
        }
        return Err("queued message request ID conflicts with a concurrently persisted row".into());
    }
    Ok(persisted)
}
