//! AI-assisted thread title generation, with fallback to a title derived
//! straight from the user's first message when generation isn't possible.

use super::support::{counts, envelope, thread_to_summary, update_thread_with_fallback_title};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
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
    title_from_user_message, title_log_fingerprint,
};

/// Whether the summarizer may replace `title`.
///
/// Two titles are ours to replace: the creation placeholder (`Chat Oct 6 …`)
/// and the interim title derived from the first user message. The interim one
/// is written the moment the user sends — before any reply exists to summarize
/// — so without this the real summary that runs on the finished reply found a
/// non-placeholder title and never ran. Anything else was typed by the user
/// and is left alone.
///
/// The interim title is only replaceable during the first exchange
/// (`agent_replies <= 1`). A summary that happens to equal the interim title
/// would otherwise look interim forever and be re-summarized on every turn.
pub(crate) fn is_replaceable_title(
    title: &str,
    first_user_message: Option<&str>,
    agent_replies: usize,
) -> bool {
    if is_auto_generated_thread_title(title) {
        return true;
    }
    agent_replies <= 1
        && first_user_message
            .and_then(title_from_user_message)
            .is_some_and(|interim| interim == title.trim())
}

/// Generates a durable thread title from the first user message and assistant reply.
pub async fn thread_generate_title(
    ctx: &DispatchCtx,
    request: GenerateConversationThreadTitleRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, ThreadsError> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let dir = config.workspace_dir.clone();
    let Some(thread) = conversations::blocking::list_threads(ctx.clone(), dir.clone())
        .await?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(ThreadsError::not_found(request.thread_id));
    };

    let messages =
        conversations::blocking::get_messages(ctx.clone(), dir.clone(), request.thread_id.clone())
            .await?;
    let first_user_message = messages
        .iter()
        .find(|message| message.sender == "user" && !message.content.trim().is_empty())
        .map(|message| message.content.trim().to_string());

    let agent_replies = messages
        .iter()
        .filter(|message| message.sender == "agent")
        .count();
    if !is_replaceable_title(&thread.title, first_user_message.as_deref(), agent_replies) {
        tracing::debug!(
            thread_id = %request.thread_id,
            title_len = thread.title.chars().count(),
            title_hash = %title_log_fingerprint(&thread.title),
            "{THREAD_TITLE_LOG_PREFIX} skipping user-chosen title"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let Some(first_user_message) = first_user_message else {
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
        let updated =
            update_thread_with_fallback_title(ctx, dir, thread, &first_user_message).await?;
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
        match provider::create_chat_model_with_model_id_in(ctx, "summarization", &config, 0.2) {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::warn!(
                    thread_id = %request.thread_id,
                    error = %error,
                    "{THREAD_TITLE_LOG_PREFIX} provider init failed; applying fallback title"
                );
                let updated =
                    update_thread_with_fallback_title(ctx, dir, thread, &first_user_message)
                        .await?;
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
                update_thread_with_fallback_title(ctx, dir, thread, &first_user_message).await?;
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
        let updated =
            update_thread_with_fallback_title(ctx, dir, thread, &first_user_message).await?;
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
        ctx.clone(),
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

#[cfg(test)]
#[path = "title_generation_tests.rs"]
mod tests;
