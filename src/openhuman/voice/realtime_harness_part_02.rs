
/// Deliver a deferred voice turn's answer into the user's in-app chat. Publishes
/// a `proactive_message` on the web-channel event bus — the same seam cron and the
/// subconscious use — which the frontend renders as an assistant message in a
/// visible thread. Web-only: it does not fan out to external channels (#5399).
fn deliver_voice_result_to_chat(correlation_id: &str, reply: String, allow_speak_back: bool) {
    let spoken = reply.trim();
    if spoken.is_empty() {
        warn!("[voice-harness] deferred turn produced no text correlation={correlation_id}");
        // The spoken ack already promised a chat follow-up, so an empty deferred
        // reply must still surface a message rather than leave the user waiting.
        deliver_voice_failure_to_chat(correlation_id);
        return;
    }
    info!(
        "[voice-harness] delivering deferred result to chat correlation={correlation_id} chars={} speak_back={allow_speak_back}",
        spoken.chars().count()
    );
    crate::openhuman::web_chat::publish_web_channel_event(crate::core::socketio::WebChannelEvent {
        event: "proactive_message".to_string(),
        client_id: VOICE_CHAT_CLIENT_ID.to_string(),
        thread_id: VOICE_CHAT_THREAD_ID.to_string(),
        full_response: Some(spoken.to_string()),
        success: Some(true),
        ..Default::default()
    });

    // Speak-back: push the finished answer to the renderer's LIVE voice session so
    // the agent can read it aloud. The frontend voice hook listens for `voice_speak`
    // and, only while the call is still open, sends it back into the ElevenLabs
    // session (a fast read-back turn). Skipped for read-back turns themselves to
    // avoid a loop; harmless if the call already ended (nobody is subscribed).
    if allow_speak_back {
        crate::openhuman::web_chat::publish_web_channel_event(
            crate::core::socketio::WebChannelEvent {
                event: "voice_speak".to_string(),
                client_id: VOICE_CHAT_CLIENT_ID.to_string(),
                full_response: Some(spoken.to_string()),
                success: Some(true),
                ..Default::default()
            },
        );
    }
}

/// Deliver a short "couldn't complete" notice to the voice chat thread for a
/// deferred turn that produced no answer the user can see — either it errored,
/// or it completed past the ack deadline with empty text (on this path
/// `run_single`'s returned text is the sole answer channel: the orchestrator
/// folds any tool/subagent output into its final reply, so an empty reply means
/// nothing was produced for the user, not that the answer went elsewhere).
/// Because the caller was told the answer was still coming, staying silent would
/// leave them waiting on a message that never arrives —
/// this makes the promised message always appear. Delivered as a normal
/// assistant message (not spoken) on the same `proactive:voice` surface as a
/// successful deferred answer.
fn deliver_voice_failure_to_chat(correlation_id: &str) {
    info!(
        "[voice-harness] delivering deferred failure notice to chat correlation={correlation_id}"
    );
    crate::openhuman::web_chat::publish_web_channel_event(crate::core::socketio::WebChannelEvent {
        event: "proactive_message".to_string(),
        client_id: VOICE_CHAT_CLIENT_ID.to_string(),
        thread_id: VOICE_CHAT_THREAD_ID.to_string(),
        full_response: Some(
            "Sorry — I couldn't finish that request just now. Please try again.".to_string(),
        ),
        success: Some(false),
        ..Default::default()
    });
}

async fn emit_event(event: &str, payload: Value) {
    match global_socket_manager() {
        Some(mgr) => {
            if let Err(e) = mgr.emit(event, payload).await {
                warn!("[voice-harness] emit {event} failed: {e}");
            }
        }
        None => warn!("[voice-harness] no socket manager; dropping {event}"),
    }
}

async fn emit_error(correlation_id: &str, message: &str) {
    emit_event(
        "voice:harness:error",
        json!({ "correlationId": correlation_id, "message": message }),
    )
    .await;
}
