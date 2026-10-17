//! Delivery of a voice turn's result (or failure) into the user's in-app chat,
//! plus the raw socket emit helpers the turn handler uses to talk back to the
//! relay.

use log::{info, warn};
use serde_json::{json, Value};

use crate::platform::socket::manager::global_socket_manager;

/// Chat thread + client id the voice turn scopes as its approval / routing
/// surface, mirroring `deliver_voice_result_to_chat`. Setting these around the
/// turn (via the chat approval context plus the explicit agent thread) is what makes the voice
/// orchestrator behave like the chat path for tools that need a *routable*
/// approval surface:
///
/// - `composio_connect` fails closed with a `[policy-denied] … needs an
///   interactive chat turn` message whenever the turn has no chat approval context
///   (see `integrations/composio/tools.rs`). On voice that message got
///   paraphrased back to the user as "your Gmail connection is throwing an auth
///   error, reconnect it" — the exact voice-only Gmail-summary failure — even
///   though the same request works in tap-and-speak (a `WebChat` turn, which
///   installs this context). With the context set, the tool reaches its
///   already-connected short-circuit and returns success instead.
/// - external_effect tool approvals raised on the `ExternalChannel` turn now
///   have a thread card to route to (the same `proactive:voice` thread where
///   deferred voice answers land) rather than silently TTL-denying.
/// - The explicit run context gives async delegation (`spawn_async_subagent`) the
///   `parent_thread_id` it requires and aligns inference logs / KV-cache with
///   the voice thread.
pub(super) const VOICE_CHAT_THREAD_ID: &str = "proactive:voice";
pub(super) const VOICE_CHAT_CLIENT_ID: &str = "system";

/// Deliver a deferred voice turn's answer into the user's in-app chat. Publishes
/// a `proactive_message` on the web-channel event bus — the same seam cron
/// uses — which the frontend renders as an assistant message in a
/// visible thread. Web-only: it does not fan out to external channels (#5399).
pub(super) fn deliver_voice_result_to_chat(
    correlation_id: &str,
    reply: String,
    allow_speak_back: bool,
) {
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
    crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
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
        crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
            event: "voice_speak".to_string(),
            client_id: VOICE_CHAT_CLIENT_ID.to_string(),
            full_response: Some(spoken.to_string()),
            success: Some(true),
            ..Default::default()
        });
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
pub(super) fn deliver_voice_failure_to_chat(correlation_id: &str) {
    info!(
        "[voice-harness] delivering deferred failure notice to chat correlation={correlation_id}"
    );
    crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
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

pub(super) async fn emit_event(event: &str, payload: Value) {
    match global_socket_manager() {
        Some(mgr) => {
            if let Err(e) = mgr.emit(event, payload).await {
                warn!("[voice-harness] emit {event} failed: {e}");
            }
        }
        None => warn!("[voice-harness] no socket manager; dropping {event}"),
    }
}

pub(super) async fn emit_error(correlation_id: &str, message: &str) {
    emit_event(
        "voice:harness:error",
        json!({ "correlationId": correlation_id, "message": message }),
    )
    .await;
}
