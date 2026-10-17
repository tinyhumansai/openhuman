use super::*;
use crate::core::runtime::DispatchCtx;

// ─────────────────────────────────────────────────────────────────────────────
// Copilot / scout streaming (Phase B) — bridge a builder/scout turn's live
// AgentProgress onto the web-channel socket, keyed by a chat thread, exactly
// like an interactive chat turn.
// ─────────────────────────────────────────────────────────────────────────────

/// Where to stream a `flows_build` / `flows_discover` turn. When present, the
/// agent's progress events (`text_delta` / `thinking_delta` / `tool_call` /
/// `tool_result` / terminal `chat_done`) are published as `WebChannelEvent`s
/// tagged with this `thread_id` — the same room the shared chat pane already
/// subscribes to and decodes — so the copilot/scout UI renders streamed text,
/// tool cards, and workflow-proposal cards live instead of spinning for the
/// whole (up to 300s) headless run.
///
/// Broadcast client id is always `"system"` (like cron-triggered runs), so
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
/// target thread (and mirrors the `TurnStates` port so the tool timeline replays
/// on reopen). The bridge task lives until the agent drops its progress sender
/// (turn end). `source` is a short trace-attribution label (e.g.
/// `"flows_build"`).
pub(super) fn attach_flow_progress_bridge(
    ctx: &DispatchCtx,
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
        crate::agent::session_store::turn_states_for(&config.workspace_dir),
        crate::web_chat::ChatRequestMetadata {
            source: Some(source.to_string()),
            ..Default::default()
        },
        config.clone(),
        ctx.clone(),
    );
}

/// Emit the terminal chat event a streamed builder/scout turn owes its viewers.
/// The progress bridge only streams intermediate deltas; without this the live
/// session spins forever.
/// streamed run: a success delivers a `chat_done` (via the shared presentation
/// path, so segmentation/reaction match a normal turn), a failure publishes a
/// `chat_error`. Broadcast as `"system"` so any viewer of the thread receives
/// it (frontend keys by `thread_id`).
pub(super) async fn finalize_flow_stream(
    ctx: &DispatchCtx,
    target: &FlowStreamTarget,
    result: &Result<String, String>,
    prompt: &str,
) {
    match result {
        Ok(text) => {
            crate::web_chat::presentation::deliver_response(
                ctx,
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
                // `attach_flow_progress_bridge` discards its
                // `ProgressBridgeHandle`, so there is no timing snapshot to
                // forward here.
                None,
                // Flow Canvas copilot streaming is not the interactive chat
                // surface follow-up suggestions are for (C5).
                false,
            )
            .await;
        }
        Err(err) => {
            crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
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
    // Settle this turn's snapshot now the turn is over. `attach_flow_progress_bridge`
    // discards its `ProgressBridgeHandle` and never waits for a drain, and the
    // bridge is otherwise the only writer that marks a snapshot terminal — it
    // does so on its way out, which can be minutes late or never. A snapshot
    // left non-terminal makes `threads_turn_state_get` report the turn as still
    // running, so re-entering the thread paints a permanent "Thinking..."
    // indicator under a reply that already landed.
    if let Ok(config) = ctx.load_config().await {
        let lifecycle = if result.is_ok() {
            tinyagents_session::turn_state::TurnLifecycle::Completed
        } else {
            tinyagents_session::turn_state::TurnLifecycle::Interrupted
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Err(err) = crate::agent::session_store::turn_states_for(&config.workspace_dir)
            .settle_turn(&target.thread_id, &target.request_id, lifecycle, &now)
        {
            tracing::warn!(
                target: "flows",
                thread_id = %target.thread_id,
                request_id = %target.request_id,
                error = %err,
                "[flows] failed to settle turn snapshot"
            );
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
