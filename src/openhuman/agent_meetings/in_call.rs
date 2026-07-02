//! In-call agency (PR-6, issue #3512).
//!
//! Handles `BackendMeetInCallRequest` events — wake-phrase commands the
//! backend Recall bot detected mid-call ("Hey Tiny, schedule a follow-up")
//! — by routing them through the FULL orchestrator agent (the same brain
//! the in-app chat uses: memory tree, connected integrations, MCP, tools)
//! and speaking the reply back into the call via the outbound `bot:speak`
//! Socket.IO event.
//!
//! Latency design:
//! - One orchestrator `Agent` is built per meeting (keyed by correlation
//!   id) and cached for the meeting's lifetime, so only the first command
//!   pays the cold build; later turns reuse in-memory history.
//! - If the orchestrator takes longer than [`ACK_AFTER_SECS`], a short
//!   spoken ack ("On it — one moment.") bridges the silence so the
//!   participant knows the bot heard them.
//!
//! Gated by either the per-meeting active-mode toggle (`listen_only = false`
//! at join, tracked in [`ACTIVE_MEETINGS`]) or the global
//! `config.meet.enable_in_call_agency` master override (default `false`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, Mutex as TokioMutex, Notify};

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::approval::{
    parse_approval_reply, ApprovalChatContext, ApprovalDecision, ApprovalGate,
    InCallApprovalContext, APPROVAL_CHAT_CONTEXT, APPROVAL_IN_CALL_CONTEXT,
};
use crate::openhuman::meet_agent::brain::strip_for_speech;
use crate::openhuman::socket::global_socket_manager;

const LOG_PREFIX: &str = "[agent_meetings::in_call]";

/// Wall-clock ceiling on one orchestrator turn. Slow Composio
/// integrations can take 60-80s, and a turn may additionally park up to
/// 120s inside the approval gate waiting for a voice/card decision
/// (issue #3513) — the ceiling must outlive park + execution, otherwise
/// the timeout would cancel a still-pending approval.
const IN_CALL_TURN_TIMEOUT_SECS: u64 = 180;

/// If the orchestrator hasn't replied within this window, speak a short
/// ack so the participant isn't left wondering whether the bot heard.
const ACK_AFTER_SECS: u64 = 3;

/// Spoken when the turn outlives [`ACK_AFTER_SECS`].
const ACK_PHRASE: &str = "On it — one moment.";

/// Spoken when the turn fails or times out. Generic on purpose: error
/// details go to logs, not into the call.
const FAILURE_PHRASE: &str = "Sorry, I couldn't finish that. I'll note it in the meeting thread.";

/// How many trailing transcript turns are included as meeting context.
const RECENT_TRANSCRIPT_WINDOW: usize = 20;

/// Voice directive baked into the per-meeting orchestrator's system
/// prompt suffix. The reply is read aloud by TTS into a live call.
const IN_CALL_VOICE_DIRECTIVE: &str = "\
You are attending a live meeting as a spoken voice assistant. A \
participant has addressed you by your wake phrase. Your reply will be \
synthesized to speech and played into the call, so: answer in one or two \
short conversational sentences; never use markdown, bullet lists, code, \
URLs, or emoji; round numbers to something speakable; if the request \
needs a tool or integration you have access to, use it rather than \
guessing. If you genuinely need clarification, ask one short question. \
Stay strictly on the participant's request — do not volunteer summaries \
or commentary on the rest of the meeting.";

/// Per-meeting orchestrator cache keyed by correlation id. Same pattern
/// as `meet_agent::brain`: the Agent's in-memory history accumulates
/// across turns so follow-up commands can reference earlier ones, and
/// later turns skip the 5-10s cold build.
static AGENT_CACHE: OnceLock<TokioMutex<HashMap<String, Arc<TokioMutex<Agent>>>>> = OnceLock::new();

fn agent_cache() -> &'static TokioMutex<HashMap<String, Arc<TokioMutex<Agent>>>> {
    AGENT_CACHE.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Per-meeting thread id cache so each exchange doesn't re-hit the
/// session store (and so meetings without a session row still reuse the
/// thread created for their first exchange).
static THREAD_CACHE: OnceLock<TokioMutex<HashMap<String, String>>> = OnceLock::new();

fn thread_cache() -> &'static TokioMutex<HashMap<String, String>> {
    THREAD_CACHE.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Meetings explicitly joined in active mode (`listen_only = false`, set via
/// the "respond when addressed" toggle in the join modal). A meeting in this
/// set dispatches in-call commands even when the global
/// `config.meet.enable_in_call_agency` default is off — the per-meeting toggle
/// is the source of truth, with the global flag acting as an always-on master
/// override. Populated by `handle_join`, cleared on `BackendMeetLeft`.
static ACTIVE_MEETINGS: OnceLock<TokioMutex<HashSet<String>>> = OnceLock::new();

fn active_meetings() -> &'static TokioMutex<HashSet<String>> {
    ACTIVE_MEETINGS.get_or_init(|| TokioMutex::new(HashSet::new()))
}

/// Mark a meeting as active-mode so its in-call commands are dispatched
/// regardless of the global agency default. Idempotent.
pub(super) async fn mark_meeting_active(correlation_id: Option<&str>) {
    let key = cache_key(correlation_id);
    if active_meetings().lock().await.insert(key.clone()) {
        tracing::info!("{LOG_PREFIX} meeting marked active (in-call agency enabled) meeting={key}");
    }
}

/// True when the meeting was joined in active mode (`listen_only = false`).
pub(super) async fn is_meeting_active(correlation_id: Option<&str>) -> bool {
    let key = cache_key(correlation_id);
    active_meetings().lock().await.contains(&key)
}

/// Drop the cached orchestrator + thread id for a finished meeting.
/// Called from the bus on `BackendMeetLeft` so a long-lived process
/// doesn't accumulate one Agent per meeting forever.
pub async fn clear_meeting_agent(correlation_id: Option<&str>) {
    let key = cache_key(correlation_id);
    if agent_cache().lock().await.remove(&key).is_some() {
        tracing::info!("{LOG_PREFIX} dropped cached agent for meeting={key}");
    }
    thread_cache().lock().await.remove(&key);
    active_meetings().lock().await.remove(&key);
}

/// Pre-build the per-meeting orchestrator when the bot joins, so the first
/// wake-phrase command doesn't pay the 5-10s cold build. Best-effort: a
/// failure just means the first turn falls back to building it lazily. Safe
/// to race with the first request — `get_or_build_agent` is idempotent
/// (whoever inserts last wins; an orphaned build is simply dropped).
pub(super) async fn prewarm_agent(correlation_id: Option<&str>) {
    let key = cache_key(correlation_id);
    if agent_cache().lock().await.contains_key(&key) {
        return; // already built (e.g. a fast first command beat us to it)
    }
    match get_or_build_agent(correlation_id).await {
        Ok(_) => tracing::info!("{LOG_PREFIX} pre-warmed orchestrator for meeting={key}"),
        Err(e) => tracing::warn!("{LOG_PREFIX} pre-warm failed for meeting={key}: {e}"),
    }
}

fn cache_key(correlation_id: Option<&str>) -> String {
    correlation_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_string()
}

/// Entry point, called from `bus.rs` (spawned — must not block the bus).
pub async fn handle_in_call_request(
    correlation_id: Option<String>,
    speaker: String,
    command_text: String,
    recent_transcript: Vec<BackendMeetTurn>,
    timestamp_ms: u64,
) {
    let command = command_text.trim();
    if command.is_empty() {
        tracing::debug!("{LOG_PREFIX} empty command_text — ignoring");
        return;
    }

    let cfg = crate::openhuman::config::Config::load_or_init().await.ok();
    let global_agency = cfg
        .as_ref()
        .map(|c| c.meet.enable_in_call_agency)
        .unwrap_or(false);
    // The per-meeting "respond when addressed" toggle (listen_only = false)
    // enables agency for just this call; the global flag is an always-on
    // master override. Either path lets the command through.
    let enabled = global_agency || is_meeting_active(correlation_id.as_deref()).await;
    if !enabled {
        tracing::info!(
            "{LOG_PREFIX} in-call request dropped (listen-only meeting and \
             config.meet.enable_in_call_agency = false) speaker={speaker} cmd_len={}",
            command.len()
        );
        return;
    }
    // Stream the reply sentence-by-sentence as the LLM generates it (default)
    // so the bot starts speaking on sentence one, vs one buffered emit.
    let streaming = cfg
        .as_ref()
        .map(|c| c.meet.in_call_streaming)
        .unwrap_or(true);

    // Voice approval channel (issue #3513): when an approval is parked on
    // this meeting and the wake-phrase command parses as a yes/no, route
    // it as the decision instead of dispatching a fresh orchestrator turn.
    // Anything that isn't a clear yes/no falls through as a new command —
    // the parked approval stays parked (card / later voice can decide it).
    if try_voice_approval_decision(correlation_id.as_deref(), command).await {
        return;
    }

    tracing::info!(
        correlation_id = ?correlation_id,
        speaker = %speaker,
        cmd_len = command.len(),
        transcript_turns = recent_transcript.len(),
        timestamp_ms = timestamp_ms,
        "{LOG_PREFIX} dispatching in-call command to orchestrator"
    );

    // Speculative ack: if the orchestrator outlives ACK_AFTER_SECS without
    // speaking, a short filler bridges the silence so the participant knows
    // the bot heard. Cancelled by the first spoken chunk (streaming) or by
    // the turn completing.
    let ack_cancel = Arc::new(Notify::new());
    let ack_cid = correlation_id.clone();
    let ack_cancel_task = ack_cancel.clone();
    let ack_task = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(ACK_AFTER_SECS)) => {
                if let Err(e) = emit_bot_filler(ACK_PHRASE, ack_cid.as_deref()).await {
                    tracing::debug!("[agent_meetings::in_call] ack emit failed: {e}");
                }
            }
            _ = ack_cancel_task.notified() => {}
        }
    });

    let reply = run_orchestrator_turn(
        correlation_id.as_deref(),
        &speaker,
        command,
        &recent_transcript,
        streaming,
        ack_cancel.clone(),
    )
    .await;

    // Turn finished — stop the ack from firing late.
    ack_cancel.notify_one();
    ack_task.abort();

    match reply {
        Ok(text) if !text.trim().is_empty() => {
            tracing::info!(
                correlation_id = ?correlation_id,
                reply_chars = text.chars().count(),
                streaming,
                "{LOG_PREFIX} orchestrator replied"
            );
            // In streaming mode the reply was already spoken sentence-by-
            // sentence during the turn; only the buffered path emits here.
            if !streaming {
                if let Err(e) = emit_bot_speak(&text, correlation_id.as_deref()).await {
                    tracing::warn!("{LOG_PREFIX} bot:speak emit failed: {e}");
                }
            }
            post_exchange_to_thread(correlation_id.as_deref(), &speaker, command, &text).await;
        }
        Ok(_) => {
            tracing::info!("{LOG_PREFIX} orchestrator returned empty reply — staying silent");
        }
        Err(e) => {
            tracing::warn!("{LOG_PREFIX} in-call turn failed: {e}");
            if let Err(e2) = emit_bot_speak(FAILURE_PHRASE, correlation_id.as_deref()).await {
                tracing::debug!("{LOG_PREFIX} failure phrase emit failed: {e2}");
            }
            // The failure phrase promises a note in the thread — keep it.
            post_exchange_to_thread(
                correlation_id.as_deref(),
                &speaker,
                command,
                &format!("(request failed: {e})"),
            )
            .await;
        }
    }
}

/// Route a spoken yes/no to a parked in-call approval. Returns `true`
/// when the command was consumed as an approval decision (the caller
/// must not dispatch it as a fresh turn)
async fn try_voice_approval_decision(correlation_id: Option<&str>, command: &str) -> bool {
    let Some(gate) = ApprovalGate::try_global() else {
        return false;
    };
    let key = cache_key(correlation_id);
    let Some(request_id) = gate.pending_for_meeting(&key) else {
        return false;
    };
    let Some(decision) = parse_voice_approval(command) else {
        tracing::info!(
            request_id = %request_id,
            "{LOG_PREFIX} approval parked but command is not a yes/no — treating as new command"
        );
        return false;
    };

    match gate.decide(&request_id, decision) {
        Ok(Some(row)) => {
            tracing::info!(
                request_id = %request_id,
                tool = %row.tool_name,
                decision = decision.as_str(),
                "{LOG_PREFIX} voice decision routed to parked approval"
            );
            let confirmation = if decision.is_approve() {
                "Okay, going ahead."
            } else {
                "Okay, I won't do that."
            };
            if let Err(e) = emit_bot_speak(confirmation, correlation_id).await {
                tracing::debug!("{LOG_PREFIX} confirmation emit failed: {e}");
            }
            true
        }
        Ok(None) => {
            // Row already decided or expired between the lookup and now.
            tracing::info!(
                request_id = %request_id,
                "{LOG_PREFIX} voice decision raced an already-decided approval"
            );
            if let Err(e) = emit_bot_speak("That request already expired.", correlation_id).await {
                tracing::debug!("{LOG_PREFIX} expired-notice emit failed: {e}");
            }
            true
        }
        Err(e) => {
            tracing::warn!("{LOG_PREFIX} voice decision failed: {e}");
            true
        }
    }
}

/// Voice-tolerant approval parsing: the strict web-chat set first, then
/// spoken synonyms. Trailing punctuation is stripped because ASR often
/// appends it ("Approve." / "go ahead!").
fn parse_voice_approval(command: &str) -> Option<ApprovalDecision> {
    let norm = command
        .trim()
        .trim_end_matches(['.', '!', '?', ','])
        .trim()
        .to_ascii_lowercase();
    if let Some(d) = parse_approval_reply(&norm) {
        return Some(d);
    }
    match norm.as_str() {
        "go ahead" | "go for it" | "do it" | "please do" | "sure" | "confirm" | "confirmed"
        | "yes please" | "sounds good" => Some(ApprovalDecision::ApproveOnce),
        "cancel" | "stop" | "don't" | "do not" | "don't do it" | "never mind" | "nevermind"
        | "no thanks" => Some(ApprovalDecision::Deny),
        _ => None,
    }
}

/// Speak the approval prompt into the call. Called from the meeting bus
/// when the gate publishes `InCallApprovalRequested`.
pub(super) async fn speak_approval_prompt(action_summary: &str, correlation_id: Option<&str>) {
    let prompt = format!(
        "I'd like to {action_summary}. Say — Hey Tiny, approve — to confirm, \
         or — Hey Tiny, deny — to cancel."
    );
    // Filler, not a terminal reply: the bot is now waiting for a spoken
    // decision, so the mascot should stay in its thinking cue.
    if let Err(e) = emit_bot_filler(&prompt, correlation_id).await {
        tracing::warn!("{LOG_PREFIX} approval prompt emit failed: {e}");
    }
}

/// Run one orchestrator turn for the meeting, with timeout. Returns the
/// speech-sanitized reply text.
async fn run_orchestrator_turn(
    correlation_id: Option<&str>,
    speaker: &str,
    command: &str,
    recent_transcript: &[BackendMeetTurn],
    streaming: bool,
    ack_cancel: Arc<Notify>,
) -> Result<String, String> {
    let agent_lock = get_or_build_agent(correlation_id).await?;
    let mut agent = agent_lock.lock().await;

    let user_message = build_turn_message(speaker, command, recent_transcript);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let key = cache_key(correlation_id);
    // Per-turn transcript filename roll — same rationale as meet_agent:
    // a kill mid-tool-call must not poison the next process's resume.
    agent.set_agent_definition_name(format!("orchestrator_incall_{key}_{now_ms}"));

    // Resolve the meeting thread up front so a parked approval's card
    // lands in the same thread as the exchange trail (dual-channel:
    // thread card + spoken prompt, issue #3513).
    let approval_thread_id = get_or_create_meeting_thread(correlation_id, &key)
        .await
        .ok();

    // Streaming: attach a progress sink so the reply is spoken sentence-by-
    // sentence as it generates. The consumer reads deltas concurrently with
    // the run; dropping the sink (set_on_progress(None)) after the run ends
    // closes the channel so the consumer flushes its tail and exits.
    let consumer = if streaming {
        let (tx, rx) = mpsc::channel::<AgentProgress>(64);
        agent.set_on_progress(Some(tx));
        let cid_owned = correlation_id.map(String::from);
        Some(tokio::spawn(stream_sentences(rx, cid_owned, ack_cancel)))
    } else {
        None
    };

    // Live-call speech is externally-sourced channel input: the approval
    // gate routes external_effect tools through the parking path. The two
    // task-locals make the park dual-channel: APPROVAL_CHAT_CONTEXT routes
    // the thread card + typed yes/no; APPROVAL_IN_CALL_CONTEXT routes the
    // spoken prompt + voice yes/no and clamps the TTL to two minutes.
    let fut = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
            channel: "meet".to_string(),
            sender: None,
            reply_target: key.clone(),
            message_id: format!("incall-{key}-{now_ms}"),
        },
        agent.run_single(&user_message),
    );
    let fut = APPROVAL_IN_CALL_CONTEXT.scope(
        InCallApprovalContext {
            meeting_key: key.clone(),
            correlation_id: correlation_id.map(String::from),
        },
        fut,
    );

    let timeout = Duration::from_secs(IN_CALL_TURN_TIMEOUT_SECS);
    let run = match approval_thread_id {
        Some(thread_id) => {
            // client_id "system" follows the task-dispatcher precedent: the
            // UI keys approval cards by thread_id for system-driven turns.
            let fut = APPROVAL_CHAT_CONTEXT.scope(
                ApprovalChatContext {
                    thread_id,
                    client_id: "system".to_string(),
                },
                fut,
            );
            tokio::time::timeout(timeout, fut).await
        }
        None => tokio::time::timeout(timeout, fut).await,
    };

    // Detach the progress sink and let the consumer drain the remaining
    // deltas + flush its tail sentence before we return.
    if streaming {
        agent.set_on_progress(None);
    }
    if let Some(handle) = consumer {
        let _ = handle.await;
    }

    let reply = match run {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return Err(format!("orchestrator run_single failed: {e}")),
        Err(_elapsed) => {
            return Err(format!(
                "orchestrator turn timed out after {IN_CALL_TURN_TIMEOUT_SECS}s"
            ));
        }
    };

    Ok(strip_for_speech(&reply))
}

/// Minimum characters before a complete sentence is flushed for speech.
/// Avoids choppy single-word emits and speaking abbreviations ("Mr.") as
/// their own sentence; short leading clauses are merged into the next one.
const MIN_SPEAK_CHARS: usize = 12;

/// Consume the agent's progress stream, speaking the visible answer one
/// sentence at a time as it arrives. `ThinkingDelta` (reasoning) is dropped
/// so the chain-of-thought is never spoken. On the first spoken chunk the
/// speculative ack is cancelled.
async fn stream_sentences(
    mut rx: mpsc::Receiver<AgentProgress>,
    correlation_id: Option<String>,
    ack_cancel: Arc<Notify>,
) {
    let mut buf = String::new();
    let mut seq: u32 = 0;
    let mut spoke = false;
    while let Some(ev) = rx.recv().await {
        if let AgentProgress::TextDelta { delta, .. } = ev {
            buf.push_str(&delta);
            for sentence in drain_sentences(&mut buf) {
                speak_stream_chunk(
                    &sentence,
                    correlation_id.as_deref(),
                    &mut seq,
                    &ack_cancel,
                    &mut spoke,
                )
                .await;
            }
        }
        // ThinkingDelta + all other progress events: ignored. Only visible
        // answer text is spoken.
    }
    // Flush the trailing partial sentence (no terminator) as the tail.
    let tail = strip_for_speech(buf.trim());
    if !tail.is_empty() {
        speak_stream_chunk(
            &tail,
            correlation_id.as_deref(),
            &mut seq,
            &ack_cancel,
            &mut spoke,
        )
        .await;
    }
}

async fn speak_stream_chunk(
    text: &str,
    correlation_id: Option<&str>,
    seq: &mut u32,
    ack_cancel: &Arc<Notify>,
    spoke: &mut bool,
) {
    if text.is_empty() {
        return;
    }
    if !*spoke {
        *spoke = true;
        ack_cancel.notify_one(); // first real audio — no need for the filler ack
    }
    if let Err(e) = emit_bot_speak_inner(text, correlation_id, Some(*seq), "reply").await {
        tracing::debug!("{LOG_PREFIX} streamed chunk emit failed: {e}");
    }
    *seq += 1;
}

/// Pull complete, speech-sanitized sentences from `buf`, leaving the trailing
/// partial sentence behind for the next delta. Short leading clauses are
/// merged into the next sentence (`MIN_SPEAK_CHARS`). Everything is held back
/// while an unclosed `<think>` tag is present so untagged reasoning from
/// models that inline it isn't spoken.
fn drain_sentences(buf: &mut String) -> Vec<String> {
    // Drop any COMPLETE reasoning blocks first, so their internal sentence
    // punctuation is never drained and spoken.
    loop {
        let (Some(open), Some(close)) = (buf.find("<think>"), buf.find("</think>")) else {
            break;
        };
        if close < open {
            break; // malformed ordering — leave it for strip_for_speech
        }
        buf.replace_range(open..close + "</think>".len(), "");
    }
    // A reasoning block is still open (no close yet) — hold everything back
    // until it closes rather than risk speaking the chain-of-thought.
    if buf.contains("<think>") {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        // Find a flush point: the earliest sentence end whose prefix is long
        // enough, merging short leading clauses into the following sentence.
        let mut flush_at = None;
        let mut search_from = 0;
        while let Some(rel) = next_sentence_end(&buf[search_from..]) {
            let abs = search_from + rel;
            if buf[..abs].trim().chars().count() >= MIN_SPEAK_CHARS {
                flush_at = Some(abs);
                break;
            }
            search_from = abs; // too short — merge with the next sentence
        }
        let Some(end) = flush_at else { break };
        let sentence: String = buf.drain(..end).collect();
        let cleaned = strip_for_speech(sentence.trim());
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out
}

/// Byte index just past the first sentence terminator (`.`/`!`/`?` followed
/// by whitespace, or a newline). Returns `None` when no boundary is present
/// yet — a terminator at the very end of the buffer waits for the next delta
/// (so "3.14" or a mid-word "." isn't treated as a boundary).
fn next_sentence_end(s: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (i, &(idx, ch)) in chars.iter().enumerate() {
        if ch == '\n' {
            return Some(idx + ch.len_utf8());
        }
        if ch == '.' || ch == '!' || ch == '?' {
            match chars.get(i + 1) {
                Some((_, next)) if next.is_whitespace() => return Some(idx + ch.len_utf8()),
                Some(_) => {}        // e.g. "3.14" — not a boundary
                None => return None, // terminator at end — wait for more
            }
        }
    }
    None
}

/// Compose the per-turn user message: live clock + meeting context +
/// the command itself. The voice directive lives in the system prompt
/// suffix (baked at agent build); only per-turn facts go here.
fn build_turn_message(
    speaker: &str,
    command: &str,
    recent_transcript: &[BackendMeetTurn],
) -> String {
    let now_local = chrono::Local::now();
    let mut msg = format!(
        "[RIGHT-NOW CONTEXT — current local time: {} ({}), tz {}. \
         Use this directly for any time/date question; do not call a tool.]\n\n",
        now_local.format("%Y-%m-%d %H:%M:%S"),
        now_local.format("%A"),
        now_local.format("%:z"),
    );

    let tail_start = recent_transcript
        .len()
        .saturating_sub(RECENT_TRANSCRIPT_WINDOW);
    let tail = &recent_transcript[tail_start..];
    if !tail.is_empty() {
        msg.push_str("[RECENT MEETING TRANSCRIPT — context only, do not reply to it]\n");
        for turn in tail {
            let content = turn.content.trim();
            if content.is_empty() {
                continue;
            }
            msg.push_str(content);
            msg.push('\n');
        }
        msg.push('\n');
    }

    msg.push_str(&format!(
        "[IN-CALL REQUEST from participant \"{speaker}\"]\n{command}"
    ));
    msg
}

/// Get the cached per-meeting orchestrator, or build it on first call.
async fn get_or_build_agent(
    correlation_id: Option<&str>,
) -> Result<Arc<TokioMutex<Agent>>, String> {
    let key = cache_key(correlation_id);
    {
        let cache = agent_cache().lock().await;
        if let Some(existing) = cache.get(&key) {
            return Ok(existing.clone());
        }
    }

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let mut agent = Agent::from_config_for_agent_with_profile(
        &config,
        "orchestrator",
        None,
        Some(IN_CALL_VOICE_DIRECTIVE.to_string()),
        // No thread-selected agent profile applies to an in-call orchestrator.
        None,
    )
    .map_err(|e| format!("{LOG_PREFIX} orchestrator build failed: {e}"))?;

    agent.set_event_context(format!("incall_{key}"), "agent_meetings");
    agent.set_agent_definition_name(format!("orchestrator_incall_{key}"));

    tracing::info!("{LOG_PREFIX} orchestrator built + cached for meeting={key}");

    let arc = Arc::new(TokioMutex::new(agent));
    agent_cache().lock().await.insert(key, arc.clone());
    Ok(arc)
}

/// Record the in-call exchange (command + spoken reply) in the meeting's
/// thread so the user has a visual trail after the call. Best-effort:
/// failures are logged, never propagated — the spoken reply already
/// happened.
async fn post_exchange_to_thread(
    correlation_id: Option<&str>,
    speaker: &str,
    command: &str,
    reply: &str,
) {
    use crate::openhuman::memory::{AppendConversationMessageRequest, ConversationMessageRecord};
    use crate::openhuman::threads::ops;

    let key = cache_key(correlation_id);
    let thread_id = match get_or_create_meeting_thread(correlation_id, &key).await {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("{LOG_PREFIX} thread lookup/create failed: {e}");
            return;
        }
    };

    let body = format!("🎙️ **{speaker}** (in-call): {command}\n\n**Tiny**: {reply}");
    let msg = ConversationMessageRecord {
        id: uuid::Uuid::new_v4().to_string(),
        content: body,
        message_type: "text".to_string(),
        extra_metadata: serde_json::Value::Null,
        sender: "system".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let append_req = AppendConversationMessageRequest {
        thread_id: thread_id.clone(),
        message: msg,
    };
    if let Err(e) = ops::message_append(append_req).await {
        tracing::warn!(
            thread_id = %thread_id,
            "{LOG_PREFIX} failed to append in-call exchange: {e}"
        );
    } else {
        tracing::info!(
            thread_id = %thread_id,
            "{LOG_PREFIX} in-call exchange posted to meeting thread"
        );
    }
}

/// Resolve the meeting's thread id: in-memory cache → session store →
/// create a fresh "Meetings"-labelled thread (and remember it for both).
async fn get_or_create_meeting_thread(
    correlation_id: Option<&str>,
    key: &str,
) -> Result<String, String> {
    use crate::openhuman::memory::CreateConversationThreadRequest;
    use crate::openhuman::threads::ops;

    if let Some(existing) = thread_cache().lock().await.get(key) {
        return Ok(existing.clone());
    }

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;

    // A calendar- or API-initiated meeting has a MeetingSession row that
    // may already carry the thread created by the transcript flow.
    if let Some(cid) = correlation_id {
        if let Ok(Some(session)) = super::store::get_session(&config, cid) {
            if let Some(tid) = session.thread_id.filter(|t| !t.is_empty()) {
                thread_cache()
                    .lock()
                    .await
                    .insert(key.to_string(), tid.clone());
                return Ok(tid);
            }
        }
    }

    let create_req = CreateConversationThreadRequest {
        labels: Some(vec!["Meetings".to_string()]),
        personality_id: None,
    };
    let outcome = ops::thread_create_new(create_req)
        .await
        .map_err(|e| format!("thread creation failed: {e}"))?;
    let thread_id = outcome
        .value
        .data
        .as_ref()
        .ok_or_else(|| "thread creation returned no data".to_string())?
        .id
        .clone();

    // Link the thread back to the session row so the post-call transcript
    // flow appends to the same thread instead of opening a second one.
    if let Some(cid) = correlation_id {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        if let Err(e) = super::store::set_session_thread_id(&config, cid, &thread_id, now_ms) {
            tracing::debug!("{LOG_PREFIX} set_session_thread_id failed (no session row?): {e}");
        }
    }

    thread_cache()
        .lock()
        .await
        .insert(key.to_string(), thread_id.clone());
    tracing::info!(
        thread_id = %thread_id,
        meeting = %key,
        "{LOG_PREFIX} created meeting thread for in-call exchanges"
    );
    Ok(thread_id)
}

/// Emit `bot:speak` to the backend over the persistent Socket.IO link.
/// The backend's SpeakOrchestrator handles streaming TTS + the audio
/// politeness gate from there.
async fn emit_bot_speak(text: &str, correlation_id: Option<&str>) -> Result<(), String> {
    emit_bot_speak_inner(text, correlation_id, None, "reply").await
}

/// Emit a non-terminal *filler* utterance (the speculative "On it" ack, an
/// approval prompt): tagged `kind="ack"` so the backend mascot returns to its
/// thinking cue afterwards instead of settling to idle — the real reply is
/// still on the way.
async fn emit_bot_filler(text: &str, correlation_id: Option<&str>) -> Result<(), String> {
    emit_bot_speak_inner(text, correlation_id, None, "ack").await
}

/// As [`emit_bot_speak`], plus an optional `seq` so the backend can order
/// the per-sentence chunks of a streamed reply, and a `kind` ("ack" filler vs
/// terminal "reply") so the backend can drive the mascot's post-speech pose
/// (both additive; older backends ignore them).
async fn emit_bot_speak_inner(
    text: &str,
    correlation_id: Option<&str>,
    seq: Option<u32>,
    kind: &str,
) -> Result<(), String> {
    let mgr = global_socket_manager()
        .ok_or_else(|| format!("{LOG_PREFIX} socket manager not initialized"))?;
    if !mgr.is_connected() {
        return Err(format!("{LOG_PREFIX} socket not connected to backend"));
    }

    let mut payload = json!({ "text": text });
    if let Some(map) = payload.as_object_mut() {
        if let Some(cid) = correlation_id {
            map.insert("correlationId".to_string(), json!(cid));
        }
        if let Some(s) = seq {
            map.insert("seq".to_string(), json!(s));
        }
        map.insert("kind".to_string(), json!(kind));
    }

    tracing::info!(
        text_len = text.len(),
        correlation_id = ?correlation_id,
        "{LOG_PREFIX} emitting bot:speak"
    );
    mgr.emit("bot:speak", payload)
        .await
        .map_err(|e| format!("{LOG_PREFIX} emit failed: {e}"))?;

    // Observability: mirror the emit on the event bus so dashboards and
    // the tracing subscriber see when the bot speaks into a call.
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::BackendMeetSpeak {
        text: text.to_string(),
        correlation_id: correlation_id.map(String::from),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> BackendMeetTurn {
        BackendMeetTurn {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn next_sentence_end_finds_terminator_before_whitespace() {
        assert_eq!(next_sentence_end("Hello there. More"), Some(12));
        assert_eq!(next_sentence_end("Done!\nNext"), Some(5));
        assert_eq!(next_sentence_end("Question? Yes"), Some(9));
    }

    #[test]
    fn next_sentence_end_waits_on_trailing_or_mid_word_dot() {
        assert_eq!(next_sentence_end("It is pi 3.14"), None); // mid-number, no boundary
        assert_eq!(next_sentence_end("No boundary yet"), None);
        assert_eq!(next_sentence_end("Trailing terminator."), None); // wait for next delta
    }

    #[test]
    fn drain_sentences_flushes_complete_sentences_and_keeps_the_tail() {
        let mut buf = String::from("The meeting is at three. We can start the");
        let out = drain_sentences(&mut buf);
        assert_eq!(out, vec!["The meeting is at three.".to_string()]);
        assert_eq!(buf, " We can start the"); // partial tail retained
    }

    #[test]
    fn drain_sentences_merges_a_short_leading_clause_into_the_next() {
        // "Yes." alone is under MIN_SPEAK_CHARS, so it merges forward.
        let mut buf = String::from("Yes. The follow-up is booked for tomorrow. ");
        let out = drain_sentences(&mut buf);
        assert_eq!(
            out,
            vec!["Yes. The follow-up is booked for tomorrow.".to_string()]
        );
    }

    #[test]
    fn drain_sentences_holds_back_unclosed_reasoning() {
        let mut buf = String::from("<think>the user wants the time. let me check.");
        assert!(drain_sentences(&mut buf).is_empty());
        // Once the tag closes, normal flushing resumes.
        buf.push_str("</think> It is three o'clock now. ");
        let out = drain_sentences(&mut buf);
        assert_eq!(out, vec!["It is three o'clock now.".to_string()]);
    }

    #[test]
    fn cache_key_falls_back_to_default() {
        assert_eq!(cache_key(None), "default");
        assert_eq!(cache_key(Some("")), "default");
        assert_eq!(cache_key(Some("   ")), "default");
        assert_eq!(cache_key(Some("meet-123")), "meet-123");
    }

    #[test]
    fn turn_message_includes_speaker_and_command() {
        let msg = build_turn_message("Alice", "schedule a follow-up tomorrow", &[]);
        assert!(msg.contains("participant \"Alice\""));
        assert!(msg.contains("schedule a follow-up tomorrow"));
        assert!(msg.contains("RIGHT-NOW CONTEXT"));
        // No transcript section when there are no turns.
        assert!(!msg.contains("RECENT MEETING TRANSCRIPT"));
    }

    #[test]
    fn turn_message_includes_transcript_tail() {
        let transcript = vec![
            turn("user", "[00:01] [Alice] welcome everyone"),
            turn("user", "[00:05] [Bob] thanks for joining"),
        ];
        let msg = build_turn_message("Alice", "what did Bob say", &transcript);
        assert!(msg.contains("RECENT MEETING TRANSCRIPT"));
        assert!(msg.contains("welcome everyone"));
        assert!(msg.contains("thanks for joining"));
    }

    #[test]
    fn turn_message_caps_transcript_window() {
        let transcript: Vec<BackendMeetTurn> = (0..50)
            .map(|i| turn("user", &format!("[00:{i:02}] [Alice] line number {i}")))
            .collect();
        let msg = build_turn_message("Alice", "summarize", &transcript);
        // Only the last RECENT_TRANSCRIPT_WINDOW turns are included.
        assert!(!msg.contains("line number 29"));
        assert!(msg.contains("line number 30"));
        assert!(msg.contains("line number 49"));
    }

    #[test]
    fn turn_message_skips_empty_transcript_lines() {
        let transcript = vec![turn("user", "   "), turn("user", "[00:01] [Alice] hello")];
        let msg = build_turn_message("Alice", "hi", &transcript);
        assert!(msg.contains("[Alice] hello"));
    }

    #[test]
    fn voice_approval_parses_strict_set_and_spoken_synonyms() {
        for approve in [
            "approve",
            "Approve.",
            "yes",
            "YES",
            "okay",
            "go ahead",
            "go ahead!",
            "do it",
            "sounds good",
            "confirmed",
        ] {
            assert_eq!(
                parse_voice_approval(approve),
                Some(ApprovalDecision::ApproveOnce),
                "{approve}"
            );
        }
        for deny in ["no", "deny", "cancel", "never mind", "don't", "stop"] {
            assert_eq!(
                parse_voice_approval(deny),
                Some(ApprovalDecision::Deny),
                "{deny}"
            );
        }
        for other in [
            "what time is it",
            "schedule a meeting",
            "approve the budget plan",
        ] {
            assert_eq!(parse_voice_approval(other), None, "{other}");
        }
    }

    #[tokio::test]
    async fn voice_decision_is_noop_without_global_gate_or_parked_approval() {
        // No global gate in unit tests → must return false (treat as a
        // normal command), never panic.
        assert!(!try_voice_approval_decision(Some("meet-x"), "approve").await);
    }

    #[tokio::test]
    async fn clear_meeting_agent_is_noop_for_unknown_meeting() {
        // Must not panic or deadlock when nothing is cached.
        clear_meeting_agent(Some("never-seen")).await;
        clear_meeting_agent(None).await;
    }

    #[tokio::test]
    async fn empty_command_returns_without_touching_config_or_socket() {
        // Empty command short-circuits before the config gate and the
        // socket — must complete instantly without panicking even in a
        // test environment with no socket manager.
        handle_in_call_request(
            Some("meet-x".into()),
            "Alice".into(),
            "   ".into(),
            vec![],
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn flag_off_drops_request_before_any_dispatch() {
        // enable_in_call_agency defaults to false; the request must be
        // dropped before the orchestrator or socket are touched. The
        // test env has no socket manager, so reaching the emit path
        // would error loudly — completing without panic proves the
        // early return.
        handle_in_call_request(
            Some("meet-flag-off".into()),
            "Alice".into(),
            "what time is it".into(),
            vec![],
            0,
        )
        .await;
        // No agent should have been built for the meeting.
        assert!(
            !agent_cache().lock().await.contains_key("meet-flag-off"),
            "agent must not be built when the flag is off"
        );
    }

    #[tokio::test]
    async fn mark_and_clear_active_meeting_toggles_membership() {
        // The per-meeting active set is what lets a listen_only=false join
        // dispatch in-call commands even with the global flag off.
        assert!(!is_meeting_active(Some("meet-active-1")).await);
        mark_meeting_active(Some("meet-active-1")).await;
        assert!(is_meeting_active(Some("meet-active-1")).await);
        // mark is idempotent.
        mark_meeting_active(Some("meet-active-1")).await;
        assert!(is_meeting_active(Some("meet-active-1")).await);
        // clear_meeting_agent must also drop the active marker so a later
        // meeting reusing the same correlation id starts passive.
        clear_meeting_agent(Some("meet-active-1")).await;
        assert!(!is_meeting_active(Some("meet-active-1")).await);
    }
}
