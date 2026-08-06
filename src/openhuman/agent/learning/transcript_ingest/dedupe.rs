//! Dedupe transcript-derived candidates against what's already stored.
//!
//! Strategy: hash the normalised candidate content and embed the hash in
//! the storage key (`<importance>.<kind>.<hash>`). Before persisting, we
//! list the existing entries in the target namespace and skip any
//! candidate whose key already exists. This is intentionally cheap — we
//! do not call `recall` (semantic) for dedupe because a fresh chat's
//! semantic recall would mask updates to the same fact.

use crate::openhuman::memory::Memory;

use super::persist;
use super::types::{ConversationReflection, MemoryCandidate};

/// Stable, deterministic content fingerprint used for dedupe.
///
/// Lower-cased, whitespace-collapsed, then hashed via FxHash. We expose
/// it as a hex string truncated to 12 chars — collisions on 48 bits are
/// astronomically unlikely for a single workspace's transcript volume,
/// and the short suffix keeps storage keys readable.
pub fn content_hash(content: &str) -> String {
    let mut normalised = String::with_capacity(content.len());
    let mut last_was_space = false;
    for ch in content.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                normalised.push(' ');
                last_was_space = true;
            }
        } else {
            for lower in ch.to_lowercase() {
                normalised.push(lower);
            }
            last_was_space = false;
        }
    }

    // FNV-1a 64-bit. Tiny, deterministic, no extra dependency.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalised.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:012x}", hash & 0x0000_ffff_ffff_ffff)
}

/// Filter out candidates that already exist in the conversation-memory
/// namespace. Returns `(kept, deduped_count)`.
pub async fn filter_new(
    memory: &dyn Memory,
    candidates: Vec<MemoryCandidate>,
) -> anyhow::Result<(Vec<MemoryCandidate>, usize)> {
    let existing = memory
        .list(
            Some(super::types::CONVERSATION_MEMORY_NAMESPACE),
            None,
            None,
        )
        .await
        .unwrap_or_default();
    let existing_keys: std::collections::HashSet<String> =
        existing.into_iter().map(|e| e.key).collect();

    let mut kept = Vec::with_capacity(candidates.len());
    let mut deduped = 0usize;
    let mut seen_in_batch: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in candidates {
        let key = persist::candidate_key(&c);
        if existing_keys.contains(&key) || !seen_in_batch.insert(key) {
            deduped += 1;
            continue;
        }
        kept.push(c);
    }
    Ok((kept, deduped))
}

/// Filter out reflections that already exist.
pub async fn filter_new_reflections(
    memory: &dyn Memory,
    reflections: Vec<ConversationReflection>,
) -> anyhow::Result<(Vec<ConversationReflection>, usize)> {
    let existing = memory
        .list(
            Some(super::types::CONVERSATION_REFLECTIONS_NAMESPACE),
            None,
            None,
        )
        .await
        .unwrap_or_default();
    let existing_keys: std::collections::HashSet<String> =
        existing.into_iter().map(|e| e.key).collect();

    let mut kept = Vec::with_capacity(reflections.len());
    let mut deduped = 0usize;
    let mut seen_in_batch: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in reflections {
        let key = persist::reflection_key(&r);
        if existing_keys.contains(&key) || !seen_in_batch.insert(key) {
            deduped += 1;
            continue;
        }
        kept.push(r);
    }
    Ok((kept, deduped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_under_whitespace_and_case() {
        let a = content_hash("I prefer Postgres for new services.");
        let b = content_hash("  i PREFER  postgres   for new services.  ");
        assert_eq!(a, b);
    }

    #[test]
    fn content_hash_differs_for_different_text() {
        let a = content_hash("I prefer Postgres for new services.");
        let b = content_hash("I prefer SQLite for new services.");
        assert_ne!(a, b);
    }
}
