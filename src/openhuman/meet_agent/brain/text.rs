//! Text post-processing: strip markdown / reasoning traces for TTS,
//! cap reply length, and build rolling dialogue history.

use super::constants::MAX_TTS_CHARS;
use super::llm::ConversationTurn;
use crate::openhuman::meet_agent::types::{SessionEvent, SessionEventKind};

/// Trim characters that sound bad when read aloud by TTS but routinely
/// leak from a chat-completions response (markdown asterisks, fenced
/// code, leading bullets). Keep punctuation that affects prosody
/// (commas, periods, question marks) intact.
pub(crate) fn strip_for_speech(text: &str) -> String {
    // Strip reasoning-model <think>...</think> blocks before we strip
    // markdown. DeepSeek / GMI / qwen-style reasoning models emit
    // their internal chain-of-thought wrapped in <think>...</think>
    // tags ahead of the user-facing reply. Without this, TTS reads
    // the entire monologue aloud — which on a 60s+ reasoning trace
    // produces a minute of bot speech the user never asked for.
    // Multiple non-overlapping blocks are stripped in sequence; an
    // unclosed <think> at the end (truncated output) drops everything
    // from the tag onwards.
    let mut cleaned = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find("<think>") {
            Some(open) => {
                cleaned.push_str(&rest[..open]);
                let after = &rest[open + "<think>".len()..];
                match after.find("</think>") {
                    Some(close) => {
                        rest = &after[close + "</think>".len()..];
                    }
                    None => {
                        // Unclosed tag → drop the rest as reasoning.
                        break;
                    }
                }
            }
            None => {
                cleaned.push_str(rest);
                break;
            }
        }
    }
    let text = cleaned.trim();

    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let cleaned: String = trimmed
            .trim_start_matches(|c: char| c == '-' || c == '*' || c == '#' || c == '>')
            .trim()
            .chars()
            .filter(|c| !matches!(c, '*' | '`' | '_' | '#'))
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&cleaned);
    }
    let trimmed = out.trim().to_string();
    let de_reasoned = strip_untagged_reasoning(&trimmed);
    cap_for_speech(&de_reasoned, MAX_TTS_CHARS)
}

/// Strip reasoning-style preamble that reasoning models leak as plain
/// text (no `<think>` tags) — phrases like "We need to generate…",
/// "I should respond with…", "The user said…", "Let me think…".
/// Heuristic: drop sentences whose lowercased trim matches a known
/// reasoning opener; if everything is reasoning, return only the last
/// sentence (final conclusion). If no signal, return input untouched.
pub(super) fn strip_untagged_reasoning(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    const REASONING_OPENERS: &[&str] = &[
        "we need to",
        "we should",
        "i need to",
        "i should",
        "i will",
        "let me ",
        "first,",
        "the user said",
        "the user is",
        "the user asked",
        "the user wants",
        "this is a",
        "this seems",
        "so i should",
        "so the response",
        "so my response",
        "okay, so",
        "alright,",
        "given that",
        "since the user",
        "the assistant",
        "the response should",
        "my response",
        "to respond",
        "responding with",
    ];
    let sentences: Vec<&str> = text
        .split_inclusive(|c: char| matches!(c, '.' | '!' | '?'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if sentences.is_empty() {
        return text.to_string();
    }
    let kept: Vec<&str> = sentences
        .iter()
        .filter(|s| {
            let lc = s.to_lowercase();
            !REASONING_OPENERS
                .iter()
                .any(|opener| lc.starts_with(opener))
        })
        .copied()
        .collect();
    if kept.is_empty() {
        // Everything was reasoning — return the last sentence as the
        // probable conclusion, lower-cased openers stripped.
        return sentences.last().map(|s| s.to_string()).unwrap_or_default();
    }
    kept.join(" ")
}

/// Truncate `text` to at most `max_chars` characters, preferring to
/// cut at the last sentence terminator (`.`, `!`, `?`) inside the
/// budget so the TTS doesn't trail off mid-clause. Falls back to a
/// hard char cut + ellipsis when no terminator fits.
pub(super) fn cap_for_speech(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max_chars).collect();
    if let Some(idx) = prefix.rfind(['.', '!', '?']) {
        let end = idx
            + prefix[idx..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
        return prefix[..end].trim_end().to_string();
    }
    let mut out = prefix.trim_end().to_string();
    out.push('…');
    out
}

/// Pull the last `window` `Heard`/`Spoke` events from the session log
/// and shape them into chat-completions turns. `Note` events are
/// internal book-keeping (errors, wake-word matches) and are skipped.
pub(crate) fn recent_dialog_history(
    events: &[SessionEvent],
    window: usize,
) -> Vec<ConversationTurn> {
    let mut out: Vec<ConversationTurn> = Vec::with_capacity(window);
    for e in events.iter().rev() {
        if out.len() >= window {
            break;
        }
        let role = match e.kind {
            SessionEventKind::Heard => "user",
            SessionEventKind::Spoke => "assistant",
            SessionEventKind::Note => continue,
        };
        let content = e.text.trim();
        if content.is_empty() {
            continue;
        }
        out.push(ConversationTurn {
            role,
            content: content.to_string(),
        });
    }
    out.reverse();
    out
}
