//! Persist memory candidates and reflections via the [`Memory`] trait.
//!
//! Storage format
//! --------------
//!
//! Conversation memory entries:
//! - **namespace**: [`super::types::CONVERSATION_MEMORY_NAMESPACE`]
//! - **key**: `<importance>.<kind>.<hash12>` — the importance prefix lets
//!   the retrieval side prune to `high.*` cheaply, and the hash dedupes.
//! - **content**: human-readable line followed by a single
//!   `[provenance] {…}` JSON line so retrievers can cite source.
//!
//! Reflections follow the same shape under
//! [`super::types::CONVERSATION_REFLECTIONS_NAMESPACE`].

use crate::openhuman::memory::{Memory, MemoryCategory};

use super::dedupe::content_hash;
use super::types::{
    ConversationReflection, MemoryCandidate, CONVERSATION_MEMORY_NAMESPACE,
    CONVERSATION_REFLECTIONS_NAMESPACE,
};

/// Compute the storage key for a candidate. Public to the module so
/// `dedupe` can reuse the exact same scheme.
pub fn candidate_key(candidate: &MemoryCandidate) -> String {
    let hash = content_hash(&candidate.content);
    format!(
        "{}.{}.{}",
        candidate.importance.as_str(),
        candidate.kind.as_str(),
        hash
    )
}

/// Compute the storage key for a reflection.
pub fn reflection_key(reflection: &ConversationReflection) -> String {
    let hash = content_hash(&format!("{}::{}", reflection.theme, reflection.detail));
    format!(
        "{}.{}.{}",
        reflection.importance.as_str(),
        reflection.theme,
        hash
    )
}

/// Render the human-readable + provenance content payload for a
/// candidate.
fn render_candidate_content(candidate: &MemoryCandidate) -> String {
    let prov_json =
        serde_json::to_string(&candidate.provenance).unwrap_or_else(|_| "{}".to_string());
    format!(
        "[{} {}] {}\n[provenance] {}",
        candidate.importance.as_str(),
        candidate.kind.as_str(),
        candidate.content,
        prov_json
    )
}

fn render_reflection_content(reflection: &ConversationReflection) -> String {
    let prov_json =
        serde_json::to_string(&reflection.provenance).unwrap_or_else(|_| "{}".to_string());
    format!(
        "[{} {}] {}\n[provenance] {}",
        reflection.importance.as_str(),
        reflection.theme,
        reflection.detail,
        prov_json
    )
}

pub async fn store_candidate(
    memory: &dyn Memory,
    candidate: &MemoryCandidate,
) -> anyhow::Result<()> {
    let key = candidate_key(candidate);
    let content = render_candidate_content(candidate);
    let session_id = candidate.provenance.thread_id.as_deref();
    memory
        .store(
            CONVERSATION_MEMORY_NAMESPACE,
            &key,
            &content,
            MemoryCategory::Conversation,
            session_id,
        )
        .await
}

pub async fn store_reflection(
    memory: &dyn Memory,
    reflection: &ConversationReflection,
) -> anyhow::Result<()> {
    let key = reflection_key(reflection);
    let content = render_reflection_content(reflection);
    let session_id = reflection.provenance.thread_id.as_deref();
    memory
        .store(
            CONVERSATION_REFLECTIONS_NAMESPACE,
            &key,
            &content,
            MemoryCategory::Conversation,
            session_id,
        )
        .await
}
