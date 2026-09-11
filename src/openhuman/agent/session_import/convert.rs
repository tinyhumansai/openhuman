//! Pure conversion helpers: stem lineage, stream naming, descriptor
//! assembly, and message-record projection.

use crate::openhuman::agent::harness::session::transcript::SessionTranscript;

use super::types::{
    DescriptorImport, DescriptorSource, DescriptorUsage, JournalMessage, SessionDescriptor,
    IMPORT_VERSION,
};

/// Parent session key from the `__` stem chain.
///
/// Stems are `{parent_chain}__{unix_ts}_{agent_id}`; the parent key is
/// everything before the **last** `__`. Roots (no `__`) have no parent.
pub fn parent_session_key(stem: &str) -> Option<String> {
    stem.rfind("__").map(|idx| stem[..idx].to_string())
}

/// Sanitize a string into the TinyAgents store-name alphabet (ASCII
/// alphanumerics, `-`, `_`, `.`); anything else becomes `_`. Empty input
/// becomes `"session"` to match the transcript layer's fallback.
pub fn sanitize_store_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.bytes().all(|b| b == b'.') {
        "session".to_string()
    } else {
        cleaned
    }
}

/// Journal stream name for a session.
///
/// Per-session streams (`session.{stem}.messages`) rather than per-thread:
/// multiple transcript files can share one `_meta.thread_id`, and appending
/// them into a shared stream would interleave sessions. The descriptor
/// carries `thread_id` so thread-level views can still be projected.
pub fn stream_name(session_key: &str) -> String {
    format!("session.{}.messages", sanitize_store_name(session_key))
}

/// Effective thread id for a transcript: `_meta.thread_id` when present,
/// otherwise a synthesized stable id. Returns `(thread_id, synthesized)`.
pub fn effective_thread_id(session_key: &str, meta_thread_id: Option<&str>) -> (String, bool) {
    match meta_thread_id {
        Some(t) if !t.is_empty() => (t.to_string(), false),
        _ => (
            format!("imported-{}", sanitize_store_name(session_key)),
            true,
        ),
    }
}

/// Build the `sessions/{session_key}` descriptor from a parsed transcript.
#[allow(clippy::too_many_arguments)]
pub fn build_descriptor(
    session_key: &str,
    transcript: &SessionTranscript,
    thread_id: String,
    thread_id_synthesized: bool,
    run_ids: Vec<String>,
    source: DescriptorSource,
    imported_at: String,
    warnings: usize,
) -> SessionDescriptor {
    let meta = &transcript.meta;
    SessionDescriptor {
        session_key: session_key.to_string(),
        parent_session_key: parent_session_key(session_key),
        thread_id,
        thread_id_synthesized,
        task_id: meta.task_id.clone(),
        run_ids,
        stream: stream_name(session_key),
        dispatcher: meta.dispatcher.clone(),
        agent_name: meta.agent_name.clone(),
        agent_id: meta.agent_id.clone(),
        agent_type: meta.agent_type.clone(),
        provider: meta.provider.clone(),
        model: meta.model.clone(),
        created: meta.created.clone(),
        updated: meta.updated.clone(),
        turn_count: meta.turn_count,
        usage: DescriptorUsage {
            input: meta.input_tokens,
            output: meta.output_tokens,
            cached_input: meta.cached_input_tokens,
            cost_usd: meta.charged_amount_usd,
        },
        source,
        import: DescriptorImport {
            version: IMPORT_VERSION,
            imported_at,
            warnings,
        },
    }
}

/// Project a transcript's messages into journal records.
pub fn journal_messages(transcript: &SessionTranscript) -> Vec<JournalMessage> {
    transcript
        .messages
        .iter()
        .map(JournalMessage::from)
        .collect()
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
