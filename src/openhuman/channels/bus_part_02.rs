
/// Post or edit a draft message carrying the latest buffered text +
/// tool status. On the first call, sends a new message and records its
/// id; on subsequent calls, edits the existing message.
async fn flush_streaming_edit(channel: &str, state: &mut StreamingState) {
    let draft = state.compose_draft();
    if draft.is_empty() {
        return;
    }
    state.dirty = false;

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };

    if let Some(ref message_id) = state.message_id {
        // Known-missing edit route: skip the guaranteed 404 and let
        // finalization replace the draft (delete + fresh atomic reply).
        if channel_edits_unsupported(channel) {
            tracing::debug!(
                "[channel-inbound][stream] skipping edit channel='{}' msg_id={} — no edit route on this backend, draft stays as-is until finalize",
                channel,
                message_id,
            );
            state.latch_draft_edits_unsupported();
            return;
        }
        let body = json!({ "text": draft });
        match client
            .send_channel_edit(channel, message_id, &jwt, body)
            .await
        {
            Ok(_) => {
                tracing::debug!(
                    "[channel-inbound][stream] edit ok channel='{}' msg_id={} chars={}",
                    channel,
                    message_id,
                    draft.len(),
                );
                state.edit_failures = 0;
            }
            Err(err) => {
                match classify_edit_failure(&err) {
                    EditFailure::RouteUnsupported => {
                        // Keep `message_id`: the draft is still on screen and
                        // finalization must be able to delete it before posting
                        // the canonical reply. Clearing it here (as the old
                        // `MessageNotFound` branch did) orphaned the draft and
                        // produced a stale "_working…_" bubble (#5230).
                        tracing::info!(
                            "[channel-inbound][stream] edit channel='{}' msg_id={} — backend has no edit route, keeping id for finalize cleanup and disabling progressive edits",
                            channel,
                            message_id,
                        );
                        mark_channel_edits_unsupported(channel);
                        state.latch_draft_edits_unsupported();
                        return;
                    }
                    EditFailure::MessageGone => {
                        tracing::info!(
                            "[channel-inbound][stream] edit channel='{}' msg_id={} — message gone provider-side (404), clearing stale id and disabling further edits",
                            channel,
                            message_id,
                        );
                        state.forget_draft();
                        return;
                    }
                    EditFailure::Transient => {}
                }
                state.edit_failures += 1;
                tracing::warn!(
                    "[channel-inbound][stream] edit failed channel='{}' msg_id={} err={} (failures={}/{})",
                    channel,
                    message_id,
                    err,
                    state.edit_failures,
                    MAX_EDIT_FAILURES,
                );
                if state.edit_failures >= MAX_EDIT_FAILURES {
                    tracing::info!(
                        "[channel-inbound][stream] giving up on progressive edits for channel='{}', falling back to atomic delivery",
                        channel,
                    );
                    state.edit_disabled = true;
                }
            }
        }
    } else {
        let body = channel_message_body_with_idempotency(channel, json!({ "text": draft }));
        match client.send_channel_message(channel, &jwt, body).await {
            Ok(resp) => {
                // A message was posted to the user — record that fact
                // *before* checking for an id. Even if we can't extract
                // one (and thus can't edit it further), we must never
                // later fall back to sending a second atomic message.
                state.draft_sent = true;
                let id = extract_message_id(&resp);
                if let Some(id) = id {
                    tracing::debug!(
                        "[channel-inbound][stream] initial draft sent channel='{}' msg_id={}",
                        channel,
                        id,
                    );
                    state.message_id = Some(id);
                } else {
                    tracing::warn!(
                        "[channel-inbound][stream] initial draft sent but response lacked id — disabling progressive edits (finalize will skip sending a duplicate) channel='{}' resp={}",
                        channel,
                        resp,
                    );
                    state.edit_disabled = true;
                }
            }
            Err(err) => {
                state.edit_failures += 1;
                tracing::warn!(
                    "[channel-inbound][stream] initial send failed channel='{}' err={} (failures={})",
                    channel,
                    err,
                    state.edit_failures,
                );
                if state.edit_failures >= MAX_EDIT_FAILURES {
                    state.edit_disabled = true;
                }
            }
        }
    }
}

/// Extract a message id from a backend `send_channel_message` response.
/// The backend has used at least three shapes: `{"id":"..."}`,
/// `{"data":{"id":"..."}}`, and `{"messageId":1456,"success":true}` —
/// the last one returns the id as a JSON number, not a string, so
/// `as_str()` alone misses it (#600).
fn extract_message_id(resp: &serde_json::Value) -> Option<String> {
    let candidate = resp
        .get("id")
        .or_else(|| resp.get("messageId"))
        .or_else(|| resp.get("data").and_then(|d| d.get("id")))
        .or_else(|| resp.get("data").and_then(|d| d.get("messageId")))?;
    if let Some(s) = candidate.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = candidate.as_i64() {
        return Some(n.to_string());
    }
    if let Some(n) = candidate.as_u64() {
        return Some(n.to_string());
    }
    None
}

/// Maximum length of the thinking snippet shown in the ephemeral
/// channel message. Longer reasoning is truncated with "…" to avoid
/// overwhelming the chat.
const MAX_THINKING_DISPLAY_CHARS: usize = 500;

/// Send or edit the ephemeral "💭 Thinking…" message on the channel.
/// This message is deleted when the final response is ready.
async fn flush_thinking_message(channel: &str, state: &mut StreamingState) {
    state.thinking_dirty = false;

    if state.thinking_accumulator.trim().is_empty() {
        return;
    }

    let mut snippet = state.thinking_accumulator.trim().to_string();
    if snippet.len() > MAX_THINKING_DISPLAY_CHARS {
        snippet.truncate(MAX_THINKING_DISPLAY_CHARS);
        snippet.push('…');
    }
    let text = format!("💭 Thinking:\n_{snippet}_");

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };

    if let Some(msg_id) = state.thinking_message_id.clone() {
        // Known-missing edit route: the bubble stays as first posted and is
        // deleted at finalization. Skip the guaranteed 404.
        if channel_edits_unsupported(channel) {
            tracing::debug!(
                "[channel-inbound][thinking] skipping edit channel='{}' msg_id={} — no edit route on this backend, bubble stays until finalize deletes it",
                channel,
                msg_id,
            );
            state.latch_thinking_edits_unsupported();
            return;
        }
        // Edit existing thinking message with updated content.
        let body = json!({ "text": text });
        if let Err(err) = client.send_channel_edit(channel, &msg_id, &jwt, body).await {
            match classify_edit_failure(&err) {
                EditFailure::RouteUnsupported => {
                    // Keep `thinking_message_id`. Clearing it (as the old
                    // `MessageNotFound` branch did) meant finalization had
                    // nothing to delete, so the ephemeral "💭 Thinking:"
                    // bubble stayed in the chat forever (#5230).
                    tracing::info!(
                        "[channel-inbound][thinking] edit channel='{}' msg_id={} — backend has no edit route, keeping id so finalize still deletes the bubble",
                        channel,
                        msg_id,
                    );
                    mark_channel_edits_unsupported(channel);
                    state.latch_thinking_edits_unsupported();
                }
                EditFailure::MessageGone => {
                    tracing::info!(
                        "[channel-inbound][thinking] edit channel='{}' msg_id={} — thinking msg gone provider-side (404), clearing id and disabling further thinking edits",
                        channel,
                        msg_id,
                    );
                    state.forget_thinking();
                }
                EditFailure::Transient => {
                    tracing::debug!(
                        "[channel-inbound][thinking] edit failed channel='{}' msg_id={} err={}",
                        channel,
                        msg_id,
                        err,
                    );
                }
            }
        }
    } else {
        // Send initial thinking message.
        let body = channel_message_body_with_idempotency(channel, json!({ "text": text }));
        match client.send_channel_message(channel, &jwt, body).await {
            Ok(resp) => {
                state.thinking_sent = true;
                let id = extract_message_id(&resp);
                if let Some(id) = id {
                    tracing::debug!(
                        "[channel-inbound][thinking] thinking msg sent channel='{}' msg_id={}",
                        channel,
                        id,
                    );
                    state.thinking_message_id = Some(id);
                } else {
                    tracing::warn!(
                        "[channel-inbound][thinking] thinking msg sent but response lacked id — disabling further thinking flushes (message won't be deletable) channel='{}' resp={}",
                        channel,
                        resp,
                    );
                    state.thinking_edit_disabled = true;
                }
            }
            Err(err) => {
                tracing::warn!(
                    "[channel-inbound][thinking] failed to send thinking msg channel='{}' err={} — disabling further thinking flushes",
                    channel,
                    err,
                );
                state.thinking_edit_disabled = true;
            }
        }
    }
}

/// Pull the most recent `MAX_FILLER_CHARS` Unicode scalars out of the
/// thinking accumulator so we can surface a live snapshot of the agent's
/// reasoning as a filler. Returns `None` when there's nothing to show
/// yet. Trims any partial leading word so the snippet reads cleanly.
fn latest_thinking_snippet(state: &StreamingState) -> Option<String> {
    let acc = state.thinking_accumulator.trim();
    if acc.is_empty() {
        return None;
    }
    let total = acc.chars().count();
    let snippet: String = if total <= MAX_FILLER_CHARS {
        acc.to_string()
    } else {
        acc.chars().skip(total - MAX_FILLER_CHARS).collect()
    };
    let trimmed = snippet
        .trim_start_matches(|c: char| !c.is_whitespace())
        .trim_start()
        .to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Post a fresh filler message to the channel and record its id so
/// `finalize_channel_reply` can delete it once the real response is on
/// screen. Prefers a live snippet of the agent's latest reasoning
/// (`thinking_accumulator`); falls back to the rotating `STATIC_FILLERS`
/// pool when there's no new thinking to show.
async fn send_filler_message(channel: &str, state: &mut StreamingState) {
    let text = match latest_thinking_snippet(state) {
        Some(snippet) if state.last_filler_snippet.as_deref() != Some(snippet.as_str()) => {
            state.last_filler_snippet = Some(snippet.clone());
            format!("💭 _{snippet}…_")
        }
        _ => {
            let pool = STATIC_FILLERS;
            let idx = state.filler_index % pool.len();
            state.filler_index = state.filler_index.wrapping_add(1);
            pool[idx].to_string()
        }
    };

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };
    let body = channel_message_body_with_idempotency(channel, json!({ "text": text }));
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            state.filler_failures = 0;
            if let Some(id) = extract_message_id(&resp) {
                tracing::debug!(
                    "[channel-inbound][filler] sent channel='{}' len={} msg_id={}",
                    channel,
                    text.len(),
                    id,
                );
                state.filler_message_ids.push(id);
            } else {
                tracing::warn!(
                    "[channel-inbound][filler] sent but response lacked id — cannot clean up on finalize channel='{}' resp={}",
                    channel,
                    resp,
                );
            }
        }
        Err(err) => {
            state.filler_failures = state.filler_failures.saturating_add(1);
            tracing::warn!(
                "[channel-inbound][filler] send failed channel='{}' err={} (failures={}/{})",
                channel,
                err,
                state.filler_failures,
                MAX_FILLER_FAILURES,
            );
            if state.filler_failures >= MAX_FILLER_FAILURES {
                tracing::info!(
                    "[channel-inbound][filler] disabling filler messages for channel='{}' — backend unsupported",
                    channel,
                );
                state.filler_disabled = true;
            }
        }
    }
}

/// Delete a previously sent message from the channel. Used to clean
/// up ephemeral thinking messages once the final response is ready.
async fn delete_channel_message(channel: &str, message_id: &str) {
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
async fn finalize_channel_reply(channel: &str, state: &mut StreamingState, final_text: &str) {
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
                            mark_channel_edits_unsupported(channel);
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
async fn build_channel_client() -> Option<(crate::api::rest::BackendOAuthClient, String)> {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
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
            tracing::error!("[channel-inbound] no session JWT — cannot send");
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
async fn send_channel_reply(channel: &str, text: &str) {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
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
            tracing::error!("[channel-inbound] no session JWT — cannot reply");
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

/// Per-sender thread-id derivation for inbound channel messages.
///
/// Matches the shape `channels::context::conversation_history_key` builds
/// for the canonical channel paths so the inbound bus handler does not
/// re-introduce a session-collapse where distinct participants in a
/// shared channel share a cached agent session.
///
/// Layout: `channel:<channel>[/<sender>][/<reply_target>][#thread:<ts>]`.
/// Each optional segment is appended only when the publisher surfaced
/// that field; legacy callers that pass only `channel` fall back to the
/// historical `channel:<channel>` key so single-DM flows keep working.
pub(crate) fn derive_inbound_thread_id(
    channel: &str,
    sender: Option<&str>,
    reply_target: Option<&str>,
    thread_ts: Option<&str>,
) -> String {
    let mut key = format!("channel:{channel}");
    let clean = |s: &str| -> Option<String> {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    if let Some(s) = sender.and_then(clean) {
        key.push('/');
        key.push_str(&s);
    }
    if let Some(r) = reply_target.and_then(clean) {
        key.push('/');
        key.push_str(&r);
    }
    // Telegram threads its messages by `thread_ts` for transport routing
    // but should not split memory/history per message — match the
    // `conversation_history_key` carve-out and skip the thread suffix
    // there. The socket layer addresses Telegram with raw channel ids
    // like `tg:123` as well as the literal `telegram` slug, so the
    // carve-out keys off whichever provider prefix the channel string
    // exposes, not the full id.
    if !channel_is_telegram(channel) {
        if let Some(t) = thread_ts.and_then(clean) {
            key.push_str("#thread:");
            key.push_str(&t);
        }
    }
    key
}

/// Build the per-turn `client_id` for an inbound socket message. Inbound
/// messages do not have a Socket.IO client id of their own — they arrive
/// from the channel transport layer rather than from a connected web
/// browser. Mint a stable label so downstream consumers that key on
/// `client_id` (the agent-turn origin, approval-chat-context, wallet
/// QuoteOwner pair, future audit-log keys) see distinct values for
/// distinct senders sharing a single Discord / Slack channel.
///
/// `None` (legacy publisher that didn't fill `sender`) maps to the bare
/// `"inbound"` literal that the path used historically, preserving
/// behavior for single-DM flows where no co-channel attacker exists.
pub(crate) fn derive_inbound_client_id(channel: &str, sender: Option<&str>) -> String {
    let trimmed_channel = channel.trim();
    let trimmed = sender.map(|s| s.trim()).filter(|s| !s.is_empty());
    match trimmed {
        Some(s) if !trimmed_channel.is_empty() => format!("inbound:{trimmed_channel}:{s}"),
        Some(s) => format!("inbound:{s}"),
        None => "inbound".to_string(),
    }
}

/// True for any inbound channel string that addresses Telegram, whether
/// the publisher uses the canonical slug (`"telegram"`) or the raw
/// provider-prefixed form the socket layer emits (`"tg:<chat_id>"`,
/// `"telegram:<chat_id>"`).
fn channel_is_telegram(channel: &str) -> bool {
    if channel == "telegram" || channel == "tg" {
        return true;
    }
    let provider = channel.split(':').next().unwrap_or("");
    matches!(provider, "telegram" | "tg")
}
