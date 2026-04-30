//! Canonicalisers — normalise source-specific payloads into canonical
//! Markdown with provenance metadata (Phase 1 / #707).
//!
//! Each source kind has its own adapter. They all return the same shape:
//! a [`CanonicalisedSource`] containing the markdown blob plus a seed
//! [`Metadata`] that the chunker will clone onto each produced chunk.
//!
//! Adapters do not interpret content semantically — they only normalise
//! shape and capture provenance. Scoring / entity extraction / summarisation
//! happen downstream in later phases.

pub mod chat;
pub mod document;
pub mod email;
pub mod email_clean;

use serde::{Deserialize, Serialize};

use crate::openhuman::memory::tree::types::{Metadata, SourceRef};

/// Output of a canonicaliser — one per logical source record
/// (a chat batch, an email, a document).
#[derive(Clone, Debug)]
pub struct CanonicalisedSource {
    pub markdown: String,
    pub metadata: Metadata,
}

/// Shared input shape: a payload + a minimal provenance hint.
///
/// Every adapter accepts this generic envelope; the concrete payload type
/// is adapter-specific (see sibling modules for the per-kind inputs).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanonicaliseRequest<P> {
    /// Logical source id (channel for chat, thread for email, doc id).
    pub source_id: String,
    /// Owner / user account.
    #[serde(default)]
    pub owner: String,
    /// Source-specific payload.
    pub payload: P,
    /// Optional tags carried through.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Trim provider-specific source references and drop blank pointers.
pub fn normalize_source_ref(source_ref: Option<String>) -> Option<SourceRef> {
    source_ref.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(SourceRef::new(trimmed.to_string()))
        }
    })
}
