//! Pure helpers for generating and validating conversation thread titles.
//!
//! Extracted from `threads::ops` so the parsing / sanitisation rules can be
//! unit-tested without pulling in `Config`, provider runtime, or RPC wiring.

use std::hash::{Hash, Hasher};

use tinyinference::message::Message;
use tinyinference::model::ModelRequest;

pub const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
pub const THREAD_TITLE_SYSTEM_PROMPT: &str = "You name chat threads from the first user message and the assistant reply. Return only the name: at most 3 words, like Fix session handoff or Gmail OAuth retry. Lead with the verb or the subject and drop filler words. No quotes. No markdown. No punctuation.";

/// Words a title carries at most. Three is the whole point of the shape: a
/// thread list is scanned, not read, and a fourth word is always the one that
/// pushes the specific words off the end of a narrow row.
pub const THREAD_TITLE_MAX_WORDS: usize = 3;
/// Hard character ceiling on a title, so one very long word cannot widen a row.
pub const THREAD_TITLE_MAX_CHARS: usize = 48;

/// Filler a title is better off without.
///
/// Prompts open with conversational scaffolding ("okay so can you please…"),
/// and taking the first three words verbatim would spend the whole title on it.
/// Only words that never identify a thread on their own are listed; a filtered
/// title that comes out empty falls back to the unfiltered words, so a message
/// made entirely of these still gets a name.
const FILLER_WORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "do", "does",
    "for", "from", "hey", "hi", "how", "i", "if", "in", "into", "is", "it", "its", "just", "let",
    "lets", "like", "me", "my", "of", "ok", "okay", "on", "or", "our", "please", "so", "thanks",
    "that", "the", "their", "then", "there", "these", "they", "this", "to", "uh", "um", "us",
    "was", "we", "well", "what", "when", "which", "will", "with", "would", "you", "your",
];

/// Stable 16-hex-char fingerprint of a title — safe for structured logs
/// where we want to correlate events without leaking the raw title text.
pub fn title_log_fingerprint(title: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Returns `true` when the title matches the auto-generated placeholder
/// shape used by `thread_create_new` (`"Chat Mon 1 1:23 AM"` / `...PM"`).
///
/// Only placeholder titles are eligible for replacement by the LLM-generated
/// title; user-renamed threads are left untouched.
pub fn is_auto_generated_thread_title(title: &str) -> bool {
    let trimmed = title.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 16 || !trimmed.starts_with("Chat ") {
        return false;
    }

    let month_end = 8;
    if bytes.len() <= month_end || !bytes[5..month_end].iter().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    if bytes.get(month_end) != Some(&b' ') {
        return false;
    }

    let mut idx = month_end + 1;
    let day_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == day_start || idx - day_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b' ') {
        return false;
    }
    idx += 1;

    let hour_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == hour_start || idx - hour_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b':') {
        return false;
    }
    idx += 1;

    if idx + 2 >= bytes.len()
        || !bytes[idx].is_ascii_digit()
        || !bytes[idx + 1].is_ascii_digit()
        || bytes[idx + 2] != b' '
    {
        return false;
    }
    idx += 3;

    matches!(&trimmed[idx..], "AM" | "PM")
}

/// Collapses any run of whitespace (including newlines/tabs) into single
/// ASCII spaces and trims the result.
pub fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reduces any text to the thread-title shape: at most
/// [`THREAD_TITLE_MAX_WORDS`] words, e.g. `Fix session handoff`.
///
/// This is the shape enforcer, not a request: the model is asked for a short
/// name in [`THREAD_TITLE_SYSTEM_PROMPT`], but a title that reaches storage as
/// a whole sentence because one completion ignored the instruction is exactly
/// the bug the shape is meant to remove, so every path runs through here.
///
/// Rules applied (in order):
/// - split on anything that is not alphanumeric (punctuation, quotes, markdown,
///   and whitespace all become word breaks)
/// - drop [`FILLER_WORDS`], unless that would leave nothing
/// - keep the first [`THREAD_TITLE_MAX_WORDS`] words, and stop early rather
///   than exceed [`THREAD_TITLE_MAX_CHARS`]
///
/// A word's own spelling is left alone — `OAuth` and `Gmail` read wrong
/// lowercased, and the model is the only thing here that knows which is which.
///
/// Returns `None` when no word survives.
pub fn shorten_title(text: &str) -> Option<String> {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let meaningful: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| !FILLER_WORDS.contains(&word.to_lowercase().as_str()))
        .collect();
    // All-filler input ("can you please") still names its thread, badly-but-
    // stably, rather than leaving the placeholder title in place.
    let source = if meaningful.is_empty() {
        words
    } else {
        meaningful
    };

    let mut title = String::new();
    for word in source.into_iter().take(THREAD_TITLE_MAX_WORDS) {
        let separator = usize::from(!title.is_empty());
        let room = THREAD_TITLE_MAX_CHARS.saturating_sub(title.chars().count() + separator);
        if room == 0 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        // A single word longer than the ceiling is truncated rather than
        // dropped: dropping it can empty an otherwise usable title.
        title.extend(word.chars().take(room));
    }
    (!title.is_empty()).then_some(title)
}

/// Sanitises a raw LLM title completion into a stored thread title.
///
/// Takes the first non-empty line — a chatty model that adds a second line of
/// commentary should not have it folded into the name — and shortens it with
/// [`shorten_title`], which absorbs the quote/markdown/punctuation stripping
/// the older sentence-shaped title needed done by hand.
///
/// Returns `None` if the result is empty.
pub fn sanitize_generated_title(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(raw);
    shorten_title(line)
}

/// Derives a stable display title directly from the first useful user message.
///
/// This is the no-provider fallback used while a conversation only has user
/// context, or when model-based title generation is unavailable. It keeps the
/// title meaningful without repeatedly renaming the thread later.
pub fn title_from_user_message(message: &str) -> Option<String> {
    let collapsed = collapse_whitespace(message);
    if collapsed.is_empty() {
        return None;
    }

    // Only the first sentence describes the ask; what follows is context the
    // title has no room for anyway.
    let first_sentence = collapsed
        .split(['.', '!', '?', '\n'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&collapsed);
    shorten_title(first_sentence)
}

/// Builds the user-visible prompt passed to the title-generation model.
pub fn build_title_prompt(user_message: &str, assistant_message: &str) -> String {
    format!(
        "First user message:\n{user_message}\n\nAssistant reply:\n{assistant_message}\n\nReturn the best thread name."
    )
}

/// Builds the whole title-generation request.
///
/// # It deliberately sets no model
///
/// The caller has already resolved the model by building the provider for the
/// `summarization` role, and the resolved model is the one that should
/// dispatch. `ModelRequest::model` is a *per-request override* that the
/// managed backend resolves verbatim, so anything set here replaces that
/// correct model on the wire.
///
/// This used to override it with `"hint:summarize"`, which no lookup table in
/// the tree defines — every hint-alias table spells the alias `summarization`.
/// The string matched nothing, survived translation unchanged, and reached the
/// backend as a literal model id, which answered
/// `400 Model 'hint:summarize' is not available` on every call. Title
/// generation then fell back to a keyword title for four months without
/// anything escalating (#5637).
///
/// Leaving `model` unset is also the only form that is correct for **every**
/// provider. `create_chat_model` resolves the `summarization` role to the
/// managed backend, a Claude Agent SDK / Claude Code model, a local runtime,
/// or a BYOK cloud slug; pinning any concrete tier id here would be wrong for
/// the four non-managed branches. Unset means each provider uses its own
/// construction-time default.
pub fn build_title_request(user_message: &str, assistant_message: &str) -> ModelRequest {
    ModelRequest::new(vec![
        Message::system(THREAD_TITLE_SYSTEM_PROMPT),
        Message::user(build_title_prompt(user_message, assistant_message)),
    ])
    .with_temperature(0.2)
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
