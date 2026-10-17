//! Test fixtures: read a thread's head transcript out of a transcript store and
//! write an edited copy back through the store port.
//!
//! Tests that need a transcript in a state the real writer never produces (an
//! orphaned tool head, a replayed `[Tool results]` round) used to splice raw
//! JSONL rows into the file the session host wrote. The host writes through a
//! store port now, so the splice goes through the same port.

use std::sync::Arc;

use tinyagents_session::port::{DriverSessionStores, SessionStoreProvider};
use tinyagents_session::transcript::{TranscriptLocator, TranscriptMessage, TranscriptMeta};

/// A fresh transcript store over the driver ports on memory (the production
/// store, minus the disk), shared by cloning the `Arc`.
pub(crate) fn memory_locator() -> Arc<dyn TranscriptLocator> {
    DriverSessionStores::new(
        Arc::new(crate::storage::MemoryStorage::new()),
        crate::storage::Scope::local(),
    )
    .expect("driver session stores over memory")
    .for_agent("fixture")
    .transcripts
}

/// A thread's head transcript as the model reads it.
pub(crate) struct Head {
    pub stem: String,
    pub meta: TranscriptMeta,
    pub messages: Vec<TranscriptMessage>,
    /// The tool declarations last recorded with the transcript.
    pub tools: Option<serde_json::Value>,
}

/// The single root transcript of `thread_id`.
///
/// # Panics
///
/// When the thread has no transcript or more than one root.
pub(crate) fn head_of(locator: &Arc<dyn TranscriptLocator>, thread_id: &str) -> Head {
    let transcripts = locator
        .thread_transcripts(thread_id, None)
        .expect("the thread has a transcript");
    assert_eq!(transcripts.roots.len(), 1, "one head transcript");
    let root = &transcripts.roots[0];
    let session = root
        .transcript
        .read_session()
        .expect("read the head transcript")
        .expect("the head transcript has a session");
    Head {
        stem: root.stem.clone(),
        meta: session.meta,
        messages: session.messages,
        tools: session.tools,
    }
}

/// Replaces the head transcript's messages with `messages`.
pub(crate) fn replace_head(
    locator: &Arc<dyn TranscriptLocator>,
    head: &Head,
    messages: &[TranscriptMessage],
) {
    locator
        .open_stem(&head.stem, head.meta.clone())
        .expect("bind the head transcript")
        .replace(messages)
        .expect("replace the head transcript");
}

/// A transcript header with nothing set, for binding a stem that already
/// exists (the store keeps the header it has).
pub(crate) fn blank_meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "fixture".into(),
        agent_id: None,
        agent_type: None,
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".into(),
        updated: "2026-01-01T00:00:00Z".into(),
        turn_count: 0,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
        task_id: None,
        session_id: None,
        parent_session_id: None,
    }
}
