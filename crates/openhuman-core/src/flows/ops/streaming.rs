use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Copilot / scout streaming (Phase B) — bridge a builder/scout turn's live
// AgentProgress onto the web-channel socket, keyed by a chat thread, exactly
// like an interactive chat turn. Blueprint: `agent/task_dispatcher/executor.rs`.
// ─────────────────────────────────────────────────────────────────────────────

/// Where to stream a `flows_build` / `flows_discover` turn. When present, the
/// agent's progress events (`text_delta` / `thinking_delta` / `tool_call` /
/// `tool_result` / terminal `chat_done`) are published as `WebChannelEvent`s
/// tagged with this `thread_id` — the same room the shared chat pane already
/// subscribes to and decodes — so the copilot/scout UI renders streamed text,
/// tool cards, and workflow-proposal cards live instead of spinning for the
/// whole (up to 300s) headless run.
///
/// Broadcast client id is always `"system"` (like cron / task-session runs), so
/// any client viewing the thread receives the events (the frontend keys by
/// `thread_id`). The blocking `{ proposal, assistant_text }` return is
/// unchanged — streaming is purely additive, opt-in per call.
#[derive(Debug, Clone)]
pub struct FlowStreamTarget {
    /// The chat thread the copilot/scout turn streams into.
    pub thread_id: String,
    /// Per-turn correlation id (matches the frontend `request_id`). Generated
    /// when the caller doesn't supply one.
    pub request_id: String,
}

impl FlowStreamTarget {
    /// Build a streaming target from optional RPC params. Streaming is enabled
    /// only when a non-empty `thread_id` is given; a missing/blank `request_id`
    /// is filled with a fresh uuid so the turn is always correlatable. Returns
    /// `None` (headless run, prior behaviour) when no usable `thread_id`.
    pub fn from_params(thread_id: Option<String>, request_id: Option<String>) -> Option<Self> {
        let thread_id = thread_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())?;
        let request_id = request_id
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Some(Self {
            thread_id,
            request_id,
        })
    }
}

/// Attach the web-channel progress bridge to `agent` for a builder/scout turn.
/// Wires an mpsc channel into the agent's progress sink and spawns the bridge
/// task that translates each [`AgentProgress`] into a socket event keyed by the
/// target thread (and mirrors a `TurnStateStore` so the tool timeline replays
/// on reopen). The bridge task lives until the agent drops its progress sender
/// (turn end). `source` is a short trace-attribution label (e.g.
/// `"flows_build"`).
pub(super) fn attach_flow_progress_bridge(
    agent: &mut crate::agent::OpenHumanSessionHost,
    target: &FlowStreamTarget,
    source: &str,
    config: &Config,
) {
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    tracing::info!(
        target: "flows",
        thread_id = %target.thread_id,
        request_id = %target.request_id,
        source = %source,
        "[flows] progress bridge: attaching (streaming copilot/scout turn)"
    );
    crate::web_chat::spawn_progress_bridge(
        progress_rx,
        "system".to_string(),
        target.thread_id.clone(),
        target.request_id.clone(),
        crate::threads::turn_state::TurnStateStore::new(config.workspace_dir.clone()),
        crate::web_chat::ChatRequestMetadata {
            source: Some(source.to_string()),
            ..Default::default()
        },
        config.clone(),
    );
}

/// Emit the terminal chat event a streamed builder/scout turn owes its viewers.
/// The progress bridge only streams intermediate deltas; without this the live
/// session spins forever. Mirrors how `task_dispatcher/executor.rs` finalizes a
/// streamed run: a success delivers a `chat_done` (via the shared presentation
/// path, so segmentation/reaction match a normal turn), a failure publishes a
/// `chat_error`. Broadcast as `"system"` so any viewer of the thread receives
/// it (frontend keys by `thread_id`).
pub(super) async fn finalize_flow_stream(
    target: &FlowStreamTarget,
    result: &Result<String, String>,
    prompt: &str,
) {
    match result {
        Ok(text) => {
            crate::web_chat::presentation::deliver_response(
                "system",
                &target.thread_id,
                &target.request_id,
                text,
                prompt,
                &[],
                // Builder/scout turns don't surface in the chat footer; their
                // token/cost spend is still captured by the global cost tracker.
                None,
                // No workspace in scope on this path, so the viewing client
                // stays the only persister of a flow turn's reply — unchanged
                // from before #6034, which covered the chat surfaces.
                None,
            )
            .await;
        }
        Err(err) => {
            crate::web_chat::publish_web_channel_event(crate::core::socketio::WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: "system".to_string(),
                thread_id: target.thread_id.clone(),
                request_id: target.request_id.clone(),
                message: Some(err.clone()),
                error_type: Some("agent_error".to_string()),
                ..Default::default()
            });
        }
    }
    tracing::info!(
        target: "flows",
        thread_id = %target.thread_id,
        request_id = %target.request_id,
        ok = result.is_ok(),
        "[flows] progress bridge: detached (terminal chat event emitted)"
    );
}
