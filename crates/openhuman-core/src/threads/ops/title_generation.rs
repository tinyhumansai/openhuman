//! AI-assisted thread title generation, with fallback to a title derived
//! straight from the user's first message when generation isn't possible.

use super::support::{counts, envelope, thread_to_summary, update_thread_with_fallback_title};
use crate::config::Config;
use crate::core::Outcome;
use crate::inference::provider;
use crate::threads::store as conversations;
use crate::threads::ThreadsError;
use crate::threads::THREAD_TITLE_LOG_PREFIX;
use crate::threads::{
    ApiEnvelope, ConversationThreadSummary, GenerateConversationThreadTitleRequest,
};
use tinyagents_harness::title::{
    build_title_request, is_auto_generated_thread_title, sanitize_generated_title,
    title_log_fingerprint,
};

/// Generates a durable thread title from the first user message and assistant reply.
pub async fn thread_generate_title(
    request: GenerateConversationThreadTitleRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, ThreadsError> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let dir = config.workspace_dir.clone();
    let Some(thread) = conversations::blocking::list_threads(dir.clone())
        .await?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(ThreadsError::not_found(request.thread_id));
    };

    if !is_auto_generated_thread_title(&thread.title) {
        tracing::debug!(
            thread_id = %request.thread_id,
            title_len = thread.title.chars().count(),
            title_hash = %title_log_fingerprint(&thread.title),
            "{THREAD_TITLE_LOG_PREFIX} skipping non-placeholder title"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let messages =
        conversations::blocking::get_messages(dir.clone(), request.thread_id.clone()).await?;
    let Some(first_user_message) = messages
        .iter()
        .find(|message| message.sender == "user" && !message.content.trim().is_empty())
        .map(|message| message.content.trim().to_string())
    else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no user message yet; skipping"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    let assistant_message = request
        .assistant_message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            messages
                .iter()
                .find(|message| message.sender == "agent" && !message.content.trim().is_empty())
                .map(|message| message.content.trim().to_string())
        });

    let Some(assistant_message) = assistant_message else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no assistant message yet; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    // `_with_model_id` rather than the plain constructor: the debug line below
    // reports the model this call actually dispatches on, and only the factory
    // knows what the `summarization` role resolved to for this configuration.
    let (chat_model, resolved_model) =
        match provider::create_chat_model_with_model_id("summarization", &config, 0.2) {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::warn!(
                    thread_id = %request.thread_id,
                    error = %error,
                    "{THREAD_TITLE_LOG_PREFIX} provider init failed; applying fallback title"
                );
                let updated =
                    update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
                return Ok(envelope(
                    thread_to_summary(updated),
                    Some(counts([("num_threads", 1)])),
                    None,
                ));
            }
        };

    tracing::debug!(
        thread_id = %request.thread_id,
        user_len = first_user_message.len(),
        assistant_len = assistant_message.len(),
        model = %resolved_model,
        "{THREAD_TITLE_LOG_PREFIX} generating thread title"
    );

    let raw_title = match chat_model
        .invoke(
            &(),
            build_title_request(&first_user_message, &assistant_message),
        )
        .await
    {
        Ok(response) => response.text(),
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} title generation failed; applying fallback title"
            );
            let updated =
                update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
            return Ok(envelope(
                thread_to_summary(updated),
                Some(counts([("num_threads", 1)])),
                None,
            ));
        }
    };

    let Some(title) = sanitize_generated_title(&raw_title) else {
        tracing::warn!(
            thread_id = %request.thread_id,
            raw_title_len = raw_title.chars().count(),
            raw_title_hash = %title_log_fingerprint(&raw_title),
            "{THREAD_TITLE_LOG_PREFIX} generated empty title after sanitization; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    if title == thread.title {
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let updated = conversations::blocking::update_thread_title(
        dir,
        request.thread_id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;

    tracing::debug!(
        thread_id = %request.thread_id,
        title_len = updated.title.chars().count(),
        title_hash = %title_log_fingerprint(&updated.title),
        "{THREAD_TITLE_LOG_PREFIX} updated thread title"
    );

    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}
