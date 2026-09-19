//! Building and running the per-turn voice orchestrator: a fresh, isolated
//! `OpenHumanSessionHost` pinned to a fast non-thinking model, seeded from the relayed
//! history, run under the hard per-turn ceiling with the same chat-scoped
//! approval surface web chat installs.

use std::time::Duration;

use log::{info, warn};
use serde_json::Value;

use crate::agent::progress::AgentProgress;
use crate::agent::session_host::OpenHumanSessionHost;
use crate::agent::turn_origin::{with_origin, AgentTurnOrigin};

use super::chat_delivery::{VOICE_CHAT_CLIENT_ID, VOICE_CHAT_THREAD_ID};
use super::prompt::messages_to_history_pairs;

/// Hard ceiling on the background turn — not on how long the caller waits, which
/// the ack deadline in the turn handler caps at ~8s.
///
/// This governs the deferred half: the orchestrator keeps working after the voice
/// turn closes, and its answer is delivered into chat and read aloud if the call
/// is still up. At 90s that ceiling was cutting real answers. Observed on staging:
/// the same "summarize my emails" request finished in ~53s on one call and was
/// still running past 90s on the next, where it aborted and the caller got a
/// failure notice instead of their summary — a hard failure caused purely by
/// Composio round-trip variance, not by anything being wrong.
///
/// Kept in step with `DEFAULT_TIMEOUT_MS` in the backend's `relayService.ts`,
/// which must stay above this so the desktop's own specific error wins the race
/// rather than the relay's generic timeout.
const TURN_TIMEOUT_SECS: u64 = 180;

/// Voice-scoped transcript namespace. Building a fresh orchestrator per turn
/// would otherwise resume the *chat* orchestrator's latest transcript by name,
/// bleeding an unrelated conversation into (or out of) the voice session. A
/// dedicated name isolates voice from chat; multi-turn context comes from the
/// relayed `messages` we seed below, not from this resume path.
const VOICE_AGENT_NAME: &str = "voice";

/// Model pinned for realtime voice turns. The cloud voice session cancels a turn
/// that has produced no spoken token in ~11-12s ("Generating the LLM response
/// took too long"), and the orchestrator's default reasoning model spends that
/// whole budget *thinking* before its first word. `chat-v1` (DeepSeek-V4-Flash,
/// thinking off) is a short-turn, tool-capable SKU: the master still routes
/// delegation through the prompt (per-turn classification is disabled — see the
/// model pin in `agent/session_host/turn/core.rs`), so tool turns keep working
/// while spoken replies start in ~1s instead of ~6s. Reasoning models are the
/// wrong tool for a latency-capped realtime channel.
const VOICE_MODEL: &str = "chat-v1";

/// Spoken-output directive appended to the orchestrator profile so replies read
/// naturally through TTS instead of as markdown.
///
/// It deliberately does NOT ask for a spoken preface before tool use any more.
/// That clause existed to get audio to the caller early, back when the relay's
/// filler was inaudible until the turn closed. It cost far more than it bought:
/// a reply carrying only text and no tool call *is* the end of a turn, so a model
/// that dutifully announced "let me pull up your inbox — I'll drop the summary in
/// your chat" ended there, and that sentence was delivered as the final answer.
/// The caller got a promise and no summary — observed live, with the model
/// echoing this doc's own former example almost verbatim.
///
/// The acknowledgement is the relay's job now (`VOICE_FILLERS` in
/// `voiceAgent.ts`, spoken ~700ms in) precisely because it does not depend on the
/// model choosing to speak first. So the directive tells the model the opposite:
/// call the tool and answer from the result.
/// Build the fresh voice orchestrator, attach the streaming sink, run one turn
/// under the hard per-turn ceiling, then detach the sink so the forwarder's
/// channel closes. Runs entirely on the background task, so the ack deadline in
/// the caller covers both the build and the model round-trips.
pub(super) async fn run_voice_turn(
    correlation_id: &str,
    messages: &[Value],
    prompt: &str,
    progress_tx: tokio::sync::mpsc::Sender<AgentProgress>,
) -> Result<String, String> {
    let mut agent = build_voice_agent(correlation_id, messages, prompt).await?;

    // Attach the streaming sink before the turn: its presence switches the harness
    // onto the true per-token streaming path, and each `AgentProgress::TextDelta`
    // is forwarded to the relay socket by `forward_reply_deltas`.
    agent.set_on_progress(Some(progress_tx));

    let outcome = run_single_with_timeout(&mut agent, correlation_id, prompt).await;

    // Detach the sink so the forwarder's channel closes the moment the turn ends,
    // deterministically rather than waiting on `agent`'s drop.
    agent.set_on_progress(None);
    outcome
}

/// Construct the per-turn voice orchestrator: load config, pin the fast voice
/// model, isolate the transcript namespace, and seed the relayed history.
async fn build_voice_agent(
    correlation_id: &str,
    messages: &[Value],
    prompt: &str,
) -> Result<OpenHumanSessionHost, String> {
    let config = crate::config::ops::load_config_with_timeout().await?;
    let mut agent = OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
        .map_err(|e| format!("orchestrator build failed: {e}"))?;
    agent.set_event_context(format!("voice_{correlation_id}"), "voice_agent");
    // Isolate the voice transcript namespace from the chat orchestrator so a
    // fresh-per-turn agent can't resume an unrelated conversation by name.
    agent.set_agent_definition_name(VOICE_AGENT_NAME);
    // Pin a fast, non-thinking model so the first spoken token lands inside the
    // realtime session's response-time ceiling (see VOICE_MODEL).
    agent.set_model_name(VOICE_MODEL);

    // Seed the authoritative prior turns the relay carries (OpenAI `messages`),
    // so follow-ups like "what about tomorrow?" keep their context. No-ops when
    // there is nothing prior to the current user message.
    let history = messages_to_history_pairs(messages);
    if let Err(e) = agent.seed_resume_from_messages(history, prompt) {
        warn!("[voice-harness] seed prior messages failed correlation={correlation_id}: {e}");
    }

    info!(
        "[voice-harness] orchestrator turn correlation={correlation_id} prompt_chars={} history_msgs={}",
        prompt.chars().count(),
        messages.len()
    );
    Ok(agent)
}

/// Run the orchestrator turn under the hard per-turn ceiling. The streaming sink
/// must already be attached; deltas flow out while this runs.
async fn run_single_with_timeout(
    agent: &mut OpenHumanSessionHost,
    correlation_id: &str,
    prompt: &str,
) -> Result<String, String> {
    // Scope the turn with the SAME chat context the web-chat path installs
    // (`APPROVAL_CHAT_CONTEXT` plus an explicit agent thread), so approval-surfaced tools
    // behave identically on voice. Without it `composio_connect` fails closed
    // for lack of a routable surface, which the model paraphrases to the user as
    // a confabulated "reconnect your Gmail" mid email-summary (#5399). See
    // VOICE_CHAT_THREAD_ID for the full rationale. Nesting mirrors web chat:
    // origin (outer) → approval context → thread id → the agent run.
    let approval_ctx = crate::security::approval::ApprovalChatContext {
        thread_id: VOICE_CHAT_THREAD_ID.to_string(),
        client_id: VOICE_CHAT_CLIENT_ID.to_string(),
    };
    agent.set_thread_id(Some(VOICE_CHAT_THREAD_ID));
    let scoped_run = agent.run_single(prompt);
    let fut = with_origin(
        AgentTurnOrigin::ExternalChannel {
            channel: "voice".to_string(),
            sender: None,
            reply_target: correlation_id.to_string(),
            message_id: format!("voice-{correlation_id}"),
        },
        crate::security::approval::APPROVAL_CHAT_CONTEXT.scope(approval_ctx, scoped_run),
    );

    match tokio::time::timeout(Duration::from_secs(TURN_TIMEOUT_SECS), fut).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(format!("orchestrator run_single failed: {e}")),
        Err(_) => Err(format!(
            "orchestrator turn timed out after {TURN_TIMEOUT_SECS}s"
        )),
    }
}
