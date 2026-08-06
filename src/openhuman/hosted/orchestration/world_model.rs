//! Device-side world-observation notes for the hosted subconscious tier.
//!
//! In the hosted-brain era the device no longer runs the wake graph, so it does
//! not compute the server's full `OrchestrationState` delta. Instead it emits a
//! compact, bounded observation note per locally-observed event (an inbound DM,
//! an executed effect). The periodic uploader batches these to
//! `POST /orchestration/v1/world-diff`, whose receipt schedules the hosted
//! subconscious steering tick.
//!
//! # Trust boundary
//!
//! A note crosses the wire (it becomes `WorldDiffEntryWire::note`). It follows
//! the same discipline as the wire allowlist: **never** key material,
//! credentials, or a local filesystem path — only a short derived observation.
//! Message bodies are summarised to a length/shape, never copied.

/// Hard cap on a note's length (chars) — bounds the crossed payload.
const NOTE_MAX_CHARS: usize = 240;

/// Build a compact world-observation note for an ingested session event. Bounded
/// and single-line so it is safe to cross the wire: the body is summarised to its
/// character count, never copied.
pub fn observe_ingest_note(session_id: &str, sender: &str, kind: &str, body: &str) -> String {
    let body = body.trim();
    let shape = if body.is_empty() {
        "empty".to_string()
    } else {
        format!("{}chars", body.chars().count())
    };
    clamp(&format!(
        "inbound {} in {} from {} ({})",
        kind_or(kind),
        session_or(session_id),
        short_handle(sender),
        shape
    ))
}

fn kind_or(kind: &str) -> &str {
    if kind.is_empty() {
        "message"
    } else {
        kind
    }
}

fn session_or(session_id: &str) -> &str {
    if session_id.is_empty() {
        "master"
    } else {
        session_id
    }
}

/// A counterpart handle is a public identifier, never a credential — but bound
/// its length so an oversized field can't bloat the note.
fn short_handle(sender: &str) -> String {
    sender.chars().take(64).collect()
}

/// Collapse newlines and clamp to [`NOTE_MAX_CHARS`] so every note is a single
/// bounded line.
fn clamp(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(NOTE_MAX_CHARS)
        .collect()
}
