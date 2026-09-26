//! Building the backend REST client, sending/deleting channel messages, and
//! finalizing a turn's reply (replacing or deleting any draft in flight).

use super::progressive_ui::{channel_edits_unsupported, classify_edit_failure, EditFailure};
use super::streaming_state::StreamingState;
use serde_json::{json, Value};

/// Attach a deterministic idempotency key to an outbound channel message
/// body, derived from the channel and message content.
pub(super) fn channel_message_body_with_idempotency(channel: &str, body: Value) -> Value {
    let intent = tinychannels::outbound_intent_from_legacy_message(channel, body);
    tinychannels::legacy_message_value_from_outbound_intent(&intent)
}

/// Delete a previously sent message from the channel. Used to clean
/// up ephemeral thinking messages once the final response is ready.
pub(super) async fn delete_channel_message(channel: &str, message_id: &str) {
    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };
    match client.send_channel_delete(channel, message_id, &jwt).await {
        Ok(_) => {
            tracing::info!(
                "[channel-inbound] deleted ephemeral msg channel='{}' msg_id={}",
                channel,
                message_id,
            );
        }
        Err(err) => {
            if let Some(crate::api::rest::BackendApiError::MessageNotFound { .. }) =
                err.downcast_ref::<crate::api::rest::BackendApiError>()
            {
                tracing::info!(
                    "[channel-inbound] delete channel='{}' msg_id={} — message already gone provider-side (404), nothing to clean up",
                    channel,
                    message_id,
                );
            } else {
                tracing::warn!(
                    "[channel-inbound] failed to delete ephemeral msg channel='{}' msg_id={} err={}",
                    channel,
                    message_id,
                    err,
                );
            }
        }
    }
}

/// Deliver the final canonical reply.
///
/// **Invariant**: if a draft message has already been posted to the
/// channel (`state.draft_sent == true`), we MUST NOT post a second
/// message — that would duplicate the visible bubble on the user's
/// side. When we have an id we attempt one last edit; when the id was
/// lost we leave the draft in place silently. The only path that
/// creates a fresh outbound message is when no draft has been posted
/// at all.
pub(super) async fn finalize_channel_reply(
    channel: &str,
    state: &mut StreamingState,
    final_text: &str,
) {
    // Deliver the canonical reply FIRST, then clean up the ephemeral
    // "💭 Thinking:" bubble. Deleting before the reply would leave the
    // chat empty for a beat; this order keeps something visible at all
    // times (#600).
    'send: {
        if let Some(ref message_id) = state.message_id {
            // Once this process knows the backend serves no edit route, the
            // "one last edit" below is a guaranteed 404 — go straight to
            // replacing the draft. Deleting it first matters: it is still on
            // screen showing partial text, so leaving it would strand a stale
            // "_working…_" bubble next to the real answer (#5230).
            if channel_edits_unsupported(channel) {
                tracing::info!(
                    "[channel-inbound] final edit skipped channel='{}' msg_id={} — no edit route on this backend, replacing the draft with a fresh atomic reply",
                    channel,
                    message_id,
                );
                let orphan = message_id.clone();
                delete_channel_message(channel, &orphan).await;
                send_channel_reply(channel, final_text).await;
                break 'send;
            }
            // We committed to a draft earlier in the turn. Always attempt
            // to edit it with the canonical reply, even when we'd
            // previously latched `edit_disabled` during the streaming
            // phase — the user is already looking at that message, so a
            // late edit attempt is still the right call. If the edit
            // fails, delete the orphan draft and send the final reply
            // as a fresh atomic message so the user always sees it.
            if let Some((client, jwt)) = build_channel_client().await {
                let body = json!({ "text": final_text });
                match client
                    .send_channel_edit(channel, message_id, &jwt, body)
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "[channel-inbound] final edit ok channel='{}' msg_id={} chars={}",
                            channel,
                            message_id,
                            final_text.len(),
                        );
                    }
                    Err(err) => match classify_edit_failure(&err) {
                        EditFailure::MessageGone => {
                            tracing::info!(
                                "[channel-inbound] final edit channel='{}' msg_id={} — draft already gone provider-side (404), sending fresh atomic reply",
                                channel,
                                message_id,
                            );
                            send_channel_reply(channel, final_text).await;
                        }
                        // Route absent (or any other failure): the draft is
                        // still on screen showing partial text, so it has to be
                        // deleted before the canonical reply lands — otherwise
                        // the user keeps a stale "_working…_" bubble alongside
                        // the real answer (#5230).
                        EditFailure::RouteUnsupported => {
                            tracing::warn!(
                                "[channel-inbound] final edit channel='{}' msg_id={} — backend has no edit route, deleting the draft and sending a fresh atomic reply",
                                channel,
                                message_id,
                            );
                            super::progressive_ui::mark_channel_edits_unsupported(channel);
                            let orphan = message_id.clone();
                            delete_channel_message(channel, &orphan).await;
                            send_channel_reply(channel, final_text).await;
                        }
                        EditFailure::Transient => {
                            tracing::warn!(
                                "[channel-inbound] final edit failed channel='{}' msg_id={} err={} — deleting orphan draft and sending fresh atomic reply so user still sees the canonical response",
                                channel,
                                message_id,
                                err,
                            );
                            let orphan = message_id.clone();
                            delete_channel_message(channel, &orphan).await;
                            send_channel_reply(channel, final_text).await;
                        }
                    },
                }
            } else {
                tracing::warn!(
                    "[channel-inbound] cannot finalize channel='{}' msg_id={} — backend client unavailable, draft left in place",
                    channel,
                    message_id,
                );
            }
            break 'send;
        }
        if state.draft_sent {
            // A draft was posted but the backend didn't return an id, so
            // we have nothing to edit. Since the draft only contains a
            // clean text prefix (or "_working…_" placeholder), sending the
            // final response as a second bubble is acceptable — leaving
            // the user without the canonical reply is worse (#600).
            tracing::warn!(
                "[channel-inbound] sending fresh reply on channel='{}' — id-less draft exists but user needs the final response",
                channel,
            );
            send_channel_reply(channel, final_text).await;
            break 'send;
        }
        // No draft exists — this is the first (and only) message for the
        // turn. Safe to send atomically.
        send_channel_reply(channel, final_text).await;
    }

    // ── Clean up ephemeral filler + thinking messages ───────────
    // Delete after the canonical reply is already on screen so the
    // chat is never momentarily empty between the two operations.
    // Fillers first (more of them, oldest-first), then the thinking
    // bubble — purely cosmetic ordering.
    let fillers = std::mem::take(&mut state.filler_message_ids);
    for id in fillers {
        delete_channel_message(channel, &id).await;
    }
    if let Some(thinking_id) = state.thinking_message_id.take() {
        delete_channel_message(channel, &thinking_id).await;
    }
}

/// Construct the REST client + session JWT shared by every outbound
/// channel call on this turn. Returns `None` and logs if either is
/// unavailable so the caller can bail quietly.
pub(super) async fn build_channel_client() -> Option<(crate::api::rest::BackendOAuthClient, String)>
{
    let config = match crate::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to load config: {}", e);
            return None;
        }
    };
    let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            // Signed out while an inbound relay message was in flight: user
            // state, not a fault — keep it out of Sentry.
            tracing::warn!("[channel-inbound] no session JWT — cannot send");
            return None;
        }
        Err(e) => {
            tracing::error!("[channel-inbound] failed to get session token: {}", e);
            return None;
        }
    };
    match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => Some((c, jwt)),
        Err(e) => {
            tracing::error!("[channel-inbound] failed to create API client: {}", e);
            None
        }
    }
}

/// Send a text reply back to a channel via the backend REST API.
pub(super) async fn send_channel_reply(channel: &str, text: &str) {
    let config = match crate::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to load config: {}", e);
            return;
        }
    };

    let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::warn!("[channel-inbound] no session JWT — cannot reply");
            return;
        }
        Err(e) => {
            tracing::error!("[channel-inbound] failed to get session token: {}", e);
            return;
        }
    };

    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to create API client: {}", e);
            return;
        }
    };

    let body = channel_message_body_with_idempotency(channel, json!({ "text": text }));
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            tracing::info!(
                "[channel-inbound] reply sent to channel='{}' response={:?}",
                channel,
                resp
            );
        }
        Err(e) => {
            tracing::error!(
                "[channel-inbound] failed to send reply to channel='{}': {}",
                channel,
                e
            );
        }
    }
}
