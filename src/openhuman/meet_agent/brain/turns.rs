//! Turn orchestration: STT → LLM → TTS → enqueue outbound PCM.
//!
//! Two entry points:
//! - [`run_turn`]: PCM-path (VAD `EndOfUtterance`). Drains the inbound
//!   buffer, runs STT, feeds the transcript to the agentic LLM, TTS's
//!   the reply, and enqueues it.
//! - [`run_caption_turn`]: Caption-path. Wired to the wake-word trigger;
//!   skips STT because the caption text is already available. Handles
//!   the pre-roll ack, grant-intent fast path, and bare-wake greeting.

use super::access::{looks_like_grant_intent, run_grant_turn};
use super::constants::{
    ACK_PHRASES, CAPTION_TURN_DELAY_MS, CONTEXT_EVENT_WINDOW, MIN_TURN_SAMPLES, PREROLL_ACK_PHRASE,
    PREROLL_SKIP_PROMPT_CHARS,
};
use super::llm::llm_meeting_agentic;
use super::speech::{stt, tts};
use super::stubs::{stub_stt, stub_tts};
use super::text::recent_dialog_history;
use crate::openhuman::meet_agent::session::registry;
use crate::openhuman::meet_agent::types::SessionEventKind;

/// Canned acknowledgements the agent speaks out loud after capturing
/// a note. Selected by hashing the prompt (deterministic, rotates across
/// the set in normal conversation).
#[allow(dead_code)]
pub(super) fn pick_ack_phrase(prompt: &str) -> &'static str {
    if prompt.trim().is_empty() {
        return "";
    }
    let h: u32 = prompt.bytes().fold(0u32, |a, b| a.wrapping_add(b as u32));
    ACK_PHRASES[(h as usize) % ACK_PHRASES.len()]
}

/// Fire one brain turn for the named session. Returns `Ok(true)` when a
/// turn actually ran, `Ok(false)` when the inbound buffer was below the
/// floor.
pub async fn run_turn(request_id: &str) -> Result<bool, String> {
    let drained = registry().with_session(request_id, |s| s.drain_inbound())?;
    if drained.len() < MIN_TURN_SAMPLES {
        log::debug!(
            "[meet-agent] skipping turn request_id={request_id} samples={}",
            drained.len()
        );
        return Ok(false);
    }

    log::info!(
        "[meet-agent] turn start request_id={request_id} samples={}",
        drained.len()
    );

    // ─── STT ────────────────────────────────────────────────────────
    let heard = match stt(&drained).await {
        Ok(text) if text.trim().is_empty() => {
            log::info!("[meet-agent] STT empty, skipping turn request_id={request_id}");
            return Ok(false);
        }
        Ok(text) => text,
        Err(err) => {
            log::warn!("[meet-agent] STT failed request_id={request_id} err={err}");
            // Record a Note so the transcript log makes the failure
            // visible to whoever's looking at logs.
            let _ = registry().with_session(request_id, |s| {
                s.record_event(
                    SessionEventKind::Note,
                    format!("STT failure (using stub): {err}"),
                );
            });
            stub_stt(&drained).await
        }
    };
    log::info!(
        "[meet-agent] STT request_id={request_id} text_chars={}",
        heard.chars().count()
    );

    // ─── LLM (agentic only; no basic-LLM fallback to avoid toolless hallucinations) ─
    let reply_text = match llm_meeting_agentic(&heard, request_id).await {
        Ok(text) => text,
        Err(agentic_err) => {
            log::warn!(
                "[meet-agent] STT-path agentic failed — speaking polite ack request_id={request_id} err={agentic_err}"
            );
            let _ = registry().with_session(request_id, |s| {
                s.record_event(
                    SessionEventKind::Note,
                    format!("agentic path failed; speaking ack: {agentic_err}"),
                );
            });
            "Let me get back to you on that.".to_string()
        }
    };

    // ─── TTS ────────────────────────────────────────────────────────
    let synthesized = if reply_text.trim().is_empty() {
        Vec::new()
    } else {
        match tts(&reply_text).await {
            Ok(samples) => samples,
            Err(err) => {
                log::warn!("[meet-agent] TTS failed request_id={request_id} err={err}");
                let _ = registry().with_session(request_id, |s| {
                    s.record_event(
                        SessionEventKind::Note,
                        format!("TTS failure (using stub): {err}"),
                    );
                });
                stub_tts(&reply_text).await
            }
        }
    };

    registry().with_session(request_id, |s| {
        s.record_event(SessionEventKind::Heard, heard.clone());
        if !reply_text.is_empty() {
            s.record_event(SessionEventKind::Spoke, reply_text.clone());
            if !synthesized.is_empty() {
                s.enqueue_outbound_pcm(&synthesized, true);
            }
        } else {
            s.record_event(
                SessionEventKind::Note,
                "agent declined to respond".to_string(),
            );
        }
        s.turn_count += 1;
    })?;

    log::info!(
        "[meet-agent] turn done request_id={request_id} reply_chars={} synth_samples={}",
        reply_text.chars().count(),
        synthesized.len()
    );
    Ok(true)
}

/// Caption-driven turn. Drains the session's pending wake-word prompt
/// (assembled by `session::note_caption`) and runs LLM → TTS → enqueue
/// outbound. Skips STT entirely — the captions are already text.
///
/// We give the user a short window (`CAPTION_TURN_DELAY_MS`) after the
/// wake word fires so multi-caption utterances ("hey openhuman …
/// what's the weather like in paris") have a chance to assemble
/// before we hit the LLM. The shell calls this on every caption
/// push that flagged the wake word; subsequent calls before the
/// delay expires are coalesced via the session's `wake_active` flag.
pub async fn run_caption_turn(request_id: &str) -> Result<bool, String> {
    // Wait briefly so a multi-fragment wake utterance ("hey openhuman
    // what's the weather like in paris" arriving as 2-3 captions) has
    // a chance to assemble before we drain the prompt.
    tokio::time::sleep(std::time::Duration::from_millis(CAPTION_TURN_DELAY_MS)).await;

    // When wake fires from a bare "hey openhuman" with no tail, the
    // session returns None from take_pending_prompt — there's nothing
    // to feed the LLM. Previously we silently bailed (`return Ok(false)`)
    // which made the bot look broken to the user. Treat empty-tail wake
    // as a "say hi back" greeting cue: synthesize a short ack so the
    // user gets audible proof that the caption→wake→speak loop is
    // wired up end-to-end.
    //
    // Also: drop any queued outbound PCM from the previous turn.
    // Reasoning-model replies can run 60+ seconds; if the user re-fires
    // the wake mid-reply we need to stop the old speech rather than
    // play the entire backlog before the new reply starts. This makes
    // the bot interruptible from the user's side.
    let (prompt, history, was_bare_wake) = match registry().with_session(request_id, |s| {
        // Mark turn as in-flight so note_caption refuses to fire new
        // wakes until run_caption_turn returns. Without this, the
        // user's continuing speech (or growing-caption re-fires)
        // spawns 20 parallel agentic turns for one question and none
        // of them complete inside the timeout.
        s.turn_in_progress = true;
        s.cancel_outbound();
        let prompt = s.take_pending_prompt();
        let history = recent_dialog_history(s.events(), CONTEXT_EVENT_WINDOW);
        (prompt, history)
    })? {
        (Some(p), h) => (p, h, false),
        (None, h) => {
            log::info!(
                "[meet-agent] caption turn bare-wake (no tail) request_id={request_id} — replying with greeting ack"
            );
            ("hello".to_string(), h, true)
        }
    };
    log::info!(
        "[meet-agent] caption turn start request_id={request_id} prompt_chars={} history_msgs={} bare_wake={}",
        prompt.chars().count(),
        history.len(),
        was_bare_wake,
    );

    // Grant-intent fast path. When the owner says "hey openhuman,
    // allow them" / "let them in" / "go ahead" after a non-owner
    // wake refusal, treat the turn as a single-shot session-level
    // grant rather than handing the prompt to the orchestrator.
    // The pending grantee was captured by `note_caption` at refusal
    // time and lives on the session for `PENDING_GRANT_WINDOW_MS`.
    if !was_bare_wake && looks_like_grant_intent(&prompt) {
        let pending = registry()
            .with_session(request_id, |s| s.take_pending_unauthorized())
            .ok()
            .flatten();
        if let Some(grantee) = pending {
            return run_grant_turn(request_id, &grantee).await;
        }
        // No pending request to grant — fall through to the normal
        // LLM path. The model can interpret "allow" however it
        // wants from there; without a pending grantee we have no
        // session-level meaning to attach to it.
        log::info!(
            "[meet-agent] grant-intent prompt detected but no pending request — falling through request_id={request_id}"
        );
    }

    // Pre-roll filler. The orchestrator + integration tools take
    // 30–60s on slow paths (Slack / Gmail / Calendar). Without an
    // immediate acoustic cue, the user assumes the bot is broken and
    // re-asks (which the turn_in_progress gate now blocks but still
    // burns the call atmosphere). Speak a 2-word ack right away and
    // enqueue with done=false so the real reply appends cleanly when
    // it lands.
    //
    // Skip pre-roll on short prompts: greetings ("hi"), checks ("can
    // you hear me", "are you there"), time questions ("what's the
    // time"), and other trivial asks the agent answers in 2-5s
    // without tools — those don't need the ack, and "On it. Yes, I
    // can hear you" sounds redundant. The 50-char threshold is a
    // rough proxy; real second-brain questions ("am I free Friday
    // afternoon for a 30 min slot") are almost always longer.
    if !was_bare_wake && prompt.chars().count() > PREROLL_SKIP_PROMPT_CHARS {
        if let Ok(ack_pcm) = tts(PREROLL_ACK_PHRASE).await {
            let _ = registry().with_session(request_id, |s| {
                s.enqueue_outbound_pcm(&ack_pcm, false);
            });
            log::info!(
                "[meet-agent] pre-roll ack queued request_id={request_id} samples={}",
                ack_pcm.len()
            );
        } else {
            log::debug!(
                "[meet-agent] pre-roll ack synth failed request_id={request_id} — skipping pre-roll"
            );
        }
    }

    // Route the turn through the FULL orchestrator agent first — it
    // owns the user's connected integrations, memory tree, MCP
    // clients and skills, so it can actually answer "is my Friday
    // free", "what did Alice say about the deploy", etc. Falls back
    // to the bare chat-completions path on orchestrator build /
    // timeout / RPC error so a config-degraded environment still
    // produces audible output instead of dead air.
    let reply_text = match llm_meeting_agentic(&prompt, request_id).await {
        Ok(text) => text,
        Err(agentic_err) => {
            // Do NOT fall back to basic LLM. The basic path has no
            // tool access, so on a calendar/slack/gmail question it
            // confidently hallucinates "I don't have access" — which
            // is the WRONG answer and worse than silence. Speak a
            // short canned "let me get back to you" ack so the user
            // knows the question was heard but the bot couldn't
            // resolve it in time, then drop the prompt. The user
            // can re-ask (turn_in_progress gate clears as we exit).
            log::warn!(
                "[meet-agent] agentic turn failed — speaking polite ack instead of toolless fallback request_id={request_id} err={agentic_err}"
            );
            let _ = registry().with_session(request_id, |s| {
                s.record_event(
                    SessionEventKind::Note,
                    format!("agentic path failed; speaking ack: {agentic_err}"),
                );
            });
            "Let me get back to you on that.".to_string()
        }
    };

    let synthesized = if reply_text.trim().is_empty() {
        Vec::new()
    } else {
        match tts(&reply_text).await {
            Ok(samples) => samples,
            Err(err) => {
                log::warn!(
                    "[meet-agent] caption-turn TTS failed request_id={request_id} err={err}"
                );
                let _ = registry().with_session(request_id, |s| {
                    s.record_event(
                        SessionEventKind::Note,
                        format!("TTS failure (using stub): {err}"),
                    );
                });
                stub_tts(&reply_text).await
            }
        }
    };

    registry().with_session(request_id, |s| {
        s.record_event(SessionEventKind::Heard, prompt.clone());
        if !reply_text.is_empty() {
            s.record_event(SessionEventKind::Spoke, reply_text.clone());
            if !synthesized.is_empty() {
                s.enqueue_outbound_pcm(&synthesized, true);
            }
        } else {
            s.record_event(
                SessionEventKind::Note,
                "agent declined to respond".to_string(),
            );
        }
        s.turn_count += 1;
        // Clear the in-flight gate so the next wake can fire. Done
        // inside the same with_session so it lands in one critical
        // section with the reply enqueue, even if the caller drops
        // the future after this point.
        s.turn_in_progress = false;
        // Stamp turn-done time so note_caption's min-turn-gap
        // backstop can suppress wakes that fire within 15s of this
        // turn's completion (caption residue / repeat questions).
        s.mark_turn_done();
    })?;

    log::info!(
        "[meet-agent] caption turn done request_id={request_id} reply_chars={} synth_samples={} reply_preview={:?}",
        reply_text.chars().count(),
        synthesized.len(),
        reply_text.chars().take(120).collect::<String>(),
    );
    Ok(true)
}
