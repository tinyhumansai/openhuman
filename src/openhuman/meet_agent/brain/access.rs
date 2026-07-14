//! Speaker authorization: intent classification, denial messages,
//! grant-intent detection, and the grant / soft-deny turn runners.

use super::speech::tts;
use super::stubs::stub_tts;
use crate::openhuman::meet_agent::session::registry;
use crate::openhuman::meet_agent::types::SessionEventKind;

// ─── Intent classification ──────────────────────────────────────────

/// Classify a non-owner caption that tripped the wake word. The
/// gate has already decided the speaker isn't authorised; this
/// picks between a friendly hi-back (greeting / pleasantry) and
/// a polite refusal (real task ask). Matching is conservative:
/// when the post-wake tail is empty OR only contains greeting
/// words, treat it as a greeting. Anything else is assumed to be
/// a task ask.
pub(crate) fn classify_unauthorized_intent(caption_text: &str) -> UnauthorizedIntent {
    // Lift the bit of text that comes after the matched wake
    // phrase so we don't get fooled by the wake itself ("hey
    // openhuman" obviously contains "hey").
    let lower = caption_text.to_ascii_lowercase();
    let wake_phrases = [
        "hey open human",
        "hi open human",
        "hello open human",
        "hey openhuman",
        "hi openhuman",
        "hello openhuman",
        "open human",
        "openhuman",
    ];
    let tail = wake_phrases
        .iter()
        .filter_map(|p| lower.find(p).map(|i| &lower[i + p.len()..]))
        .next()
        .unwrap_or(&lower);
    // Strip punctuation / common filler so "hi there!" reduces to
    // ["hi", "there"]. Keeping the word list cheap and English-only
    // for v1; the locale-aware story lands with multilingual TTS.
    let words: Vec<&str> = tail
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return UnauthorizedIntent::Greeting;
    }
    const GREETING_WORDS: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "yo",
        "sup",
        "howdy",
        "greetings",
        "hola",
        "good",
        "morning",
        "afternoon",
        "evening",
        "night",
        "there",
        "everyone",
        "all",
        "folks",
        "team",
        "guys",
        "yall",
    ];
    if words.iter().all(|w| GREETING_WORDS.contains(w)) {
        UnauthorizedIntent::Greeting
    } else {
        UnauthorizedIntent::TaskAsk
    }
}

/// Output of `classify_unauthorized_intent`. Drives whether the
/// non-owner turn speaks a canned hi-back or routes the prompt
/// through a toolless LLM (general-knowledge + safe deflection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnauthorizedIntent {
    /// Just a greeting — bot says hi back without offering tools.
    Greeting,
    /// Substantive question. Route to a toolless LLM with a strict
    /// system prompt — answer general knowledge / casual chat,
    /// refuse anything that would require the owner's personal
    /// tools or data, and point the owner at the magic word
    /// ("allow") if access is needed.
    TaskAsk,
}

// ─── Message builders ───────────────────────────────────────────────

/// System prompt for the non-owner branch. The LLM has no tool
/// surface attached and is told to refuse any request that would
/// need the owner's personal data. Kept short and explicit so the
/// model doesn't ad-lib a different boundary.
pub(super) fn non_owner_system_prompt(owner: &str) -> String {
    let owner_label = if owner.trim().is_empty() {
        "the meeting host"
    } else {
        owner.trim()
    };
    format!(
        "\
You are openhuman, an AI participant in a live Google Meet call. The speaker is NOT the call \
owner — the owner is {owner_label}.\n\
\n\
WHAT YOU MAY DO:\n\
- Answer general knowledge questions (history, science, math, definitions, weather concepts).\n\
- Casual conversation, jokes, small talk, greetings.\n\
- Explain what you are and what you can do at a high level.\n\
\n\
WHAT YOU MUST REFUSE (no exceptions):\n\
- Anything that would require {owner_label}'s personal data: their Slack, Gmail, Calendar, \
contacts, memory notes, files, schedule, integrations, or chat history.\n\
- Sending messages, scheduling, reminding, creating, modifying or deleting any data on their \
behalf.\n\
- Revealing what {owner_label} has previously told you or stored with you.\n\
\n\
WHEN REFUSING: respond with exactly one short sentence pointing at the magic word, e.g. \
\"That needs {owner_label}'s permission — {owner_label}, say 'allow' if you'd like me to help.\"\n\
\n\
OUTPUT FORMAT (strict):\n\
- ONE short spoken sentence, max 25 words.\n\
- Plain English. No markdown, bullets, code fences, or URLs.\n\
- No meta-narration (\"I should…\", \"Let me…\", \"As an AI…\"). Just answer.\n\
- Respond in ENGLISH ONLY regardless of the speaker's language — TTS is English-only.\n\
"
    )
}

/// Friendly hi-back canned line when a non-owner just greets the
/// bot. Kept short and warm; doesn't mention the owner / privacy
/// gate at all — that's noise on a "hello".
pub(super) fn friendly_greeting_message(asker: &str) -> String {
    let asker = asker.trim();
    if asker.is_empty() {
        "Hi there! Nice to meet you.".to_string()
    } else {
        format!("Hi {asker}! Nice to meet you.")
    }
}

/// Spoken refusal when a non-owner trips the wake word. Built per
/// call from the configured owner display name so the audible
/// response names the actual person who has the keys, and tells
/// the owner the magic word ("allow") to grant access. Kept short
/// so it doesn't drown the conversation.
pub(crate) fn soft_deny_message(asker: &str, owner: &str) -> String {
    let asker = asker.trim();
    let owner = owner.trim();
    match (asker.is_empty(), owner.is_empty()) {
        (true, true) => "Sorry, I only respond to my owner.".to_string(),
        (true, false) => format!(
            "Sorry, only {owner} can ask me things in this call. {owner}, say 'allow' if you'd like me to answer."
        ),
        (false, true) => format!("Sorry {asker}, I only respond to my owner."),
        (false, false) => format!(
            "Sorry {asker}, only {owner} can ask me things here. {owner}, say 'allow' to let them in."
        ),
    }
}

// ─── Grant-intent detection ─────────────────────────────────────────

/// Recognise an "open the gate" intent from the owner's first words
/// after the wake phrase. Conservative: only fires when the prompt
/// begins with one of the canonical permit verbs so an unrelated
/// owner query that happens to contain "allow" or "yes" deeper in
/// the sentence isn't hijacked.
///
/// Returns `true` when the owner is explicitly granting access to
/// the most-recently-refused asker. The caller still gates on
/// session-level state (`take_pending_unauthorized`) — without a
/// pending request the intent is meaningless and the prompt should
/// just run as a normal LLM turn.
pub(crate) fn looks_like_grant_intent(prompt: &str) -> bool {
    let p = prompt.trim().to_ascii_lowercase();
    if p.is_empty() {
        return false;
    }
    // Whole-prompt matches first so short approvals ("allow", "yes")
    // don't collide with longer prompts that happen to start with
    // the same word.
    matches!(
        p.as_str(),
        "allow" | "yes" | "ok" | "okay" | "go ahead" | "let them in" | "let them ask" | "permit"
    ) || p.starts_with("allow ")
        || p.starts_with("let them")
        || p.starts_with("let him")
        || p.starts_with("let her")
        || p.starts_with("go ahead")
        || p.starts_with("yes go ahead")
        || p.starts_with("yes let")
        || p.starts_with("permit ")
        || p.starts_with("you can answer")
        || p.starts_with("you can tell")
}

// ─── Turn runners ────────────────────────────────────────────────────

/// Owner-grant path: the owner said "allow them" / "go ahead" /
/// "let them in" after a non-owner's wake refusal. Add the
/// previously-refused speaker to the per-call allowlist (so their
/// next wake fires through to the orchestrator), and speak a
/// short confirmation so they know they're in.
pub async fn run_grant_turn(request_id: &str, grantee: &str) -> Result<bool, String> {
    let grantee = grantee.trim();
    let message = if grantee.is_empty() {
        "Okay, you can ask me now.".to_string()
    } else {
        format!("Okay, {grantee} can ask me now.")
    };
    log::info!("[meet-agent] grant request_id={request_id} grantee=\"{grantee}\"");
    // Apply the grant on the session BEFORE speaking — if TTS races
    // and the grantee re-asks during synthesis, we want their next
    // wake to fire through. Also cancel any prior outbound so the
    // confirmation doesn't queue behind a half-drained refusal.
    let _ = registry().with_session(request_id, |s| {
        s.allow_speaker(grantee);
        s.cancel_outbound();
    });
    let samples = match tts(&message, None).await {
        Ok(samples) => samples,
        Err(err) => {
            log::warn!("[meet-agent] grant TTS failed request_id={request_id} err={err}");
            stub_tts(&message).await
        }
    };
    registry().with_session(request_id, |s| {
        s.record_event(
            SessionEventKind::Note,
            format!("owner granted wake access to {grantee}"),
        );
        s.record_event(SessionEventKind::Spoke, message.clone());
        if !samples.is_empty() {
            s.enqueue_outbound_pcm(&samples, true);
        }
        // Clear the wake_active + turn_in_progress flags so the
        // next caption (likely the grantee's actual question) can
        // fire a new turn. Without this, the wake state from the
        // owner's "allow them" prompt would coalesce the grantee's
        // first real caption into a continuation of this grant turn.
        s.wake_active = false;
        s.turn_in_progress = false;
        s.mark_turn_done();
    })?;
    Ok(true)
}

/// Soft-deny path: kick a canned-line TTS reply when the wake word
/// fires from a non-owner. Branches on intent: a bare greeting gets
/// a friendly hi-back; a substantive task ask gets the refusal that
/// tells the owner how to grant access. Does NOT touch the
/// orchestrator agent (no tool calls, no memory writes) — it's a
/// single canned line, so the failure modes are limited to TTS errors.
///
/// `caption_text` is the full caption from `note_caption` so we can
/// classify intent here; the session has already recorded the
/// pending grant request and dispatch timestamp.
pub async fn run_soft_deny_turn(
    request_id: &str,
    asker: &str,
    caption_text: &str,
) -> Result<bool, String> {
    let owner = registry()
        .with_session(request_id, |s| s.owner_display_name().to_string())
        .unwrap_or_default();
    let intent = classify_unauthorized_intent(caption_text);
    // Greeting → canned hi (no network round-trip needed).
    // TaskAsk  → toolless LLM. The LLM has no tools attached, has
    //            an explicit "refuse personal-data asks" system
    //            prompt, and is asked to point the owner at the
    //            magic word when refusing. So a Q like "what's
    //            the capital of France" lands as a normal answer
    //            ("Paris"), while "read Nikhil's Slack" lands as
    //            the refusal. The LLM picks; we don't classify.
    let message = match intent {
        UnauthorizedIntent::Greeting => friendly_greeting_message(asker),
        UnauthorizedIntent::TaskAsk => match llm_general_no_tools(caption_text, &owner).await {
            Ok(reply) if !reply.trim().is_empty() => reply,
            Ok(_) => {
                // Empty reply = LLM declined silently. Fall back to
                // the explicit canned refusal so the speaker hears
                // *something* and knows the bot didn't crash.
                log::info!(
                    "[meet-agent] non-owner LLM returned empty — using canned refusal request_id={request_id}"
                );
                soft_deny_message(asker, &owner)
            }
            Err(err) => {
                log::warn!("[meet-agent] non-owner LLM failed request_id={request_id} err={err}");
                soft_deny_message(asker, &owner)
            }
        },
    };
    log::info!(
        "[meet-agent] soft-deny request_id={request_id} asker=\"{asker}\" owner=\"{owner}\" intent={intent:?}"
    );
    // Cancel any prior outbound so the refusal doesn't queue behind a
    // half-drained reply from a previous turn.
    let _ = registry().with_session(request_id, |s| s.cancel_outbound());
    let samples = match tts(&message, None).await {
        Ok(samples) => samples,
        Err(err) => {
            log::warn!("[meet-agent] soft-deny TTS failed request_id={request_id} err={err}");
            stub_tts(&message).await
        }
    };
    registry().with_session(request_id, |s| {
        let kind = match intent {
            UnauthorizedIntent::Greeting => "greeting",
            UnauthorizedIntent::TaskAsk => "refusal",
        };
        s.record_event(
            SessionEventKind::Note,
            format!("soft-deny ({kind}): {asker} unauthorised wake"),
        );
        s.record_event(SessionEventKind::Spoke, message.clone());
        if !samples.is_empty() {
            s.enqueue_outbound_pcm(&samples, true);
        }
        // NB: do NOT call `mark_turn_done` here — that's the
        // owner-min-turn-gap stamp, and we want the owner to be
        // able to wake (e.g. say "allow them") within seconds of a
        // refusal. The session's own `UNAUTHORIZED_COOLDOWN_MS` is
        // what guards against a soft-deny loop from the same
        // non-owner speaker.
    })?;
    Ok(true)
}

// ─── Non-owner LLM path ─────────────────────────────────────────────

/// Route a non-owner caption through the toolless chat-v1 LLM.
/// Returns the spoken text — the caller TTS's it and enqueues.
async fn llm_general_no_tools(prompt: &str, owner: &str) -> Result<String, String> {
    let system_prompt = non_owner_system_prompt(owner);
    // No rolling history for the non-owner path — each ask is a
    // fresh conversation. Sharing history between owner turns and
    // non-owner turns risks leaking the owner's tool-call results
    // into a stranger-facing reply.
    super::llm::llm_meeting_basic(prompt, &[], &system_prompt).await
}
