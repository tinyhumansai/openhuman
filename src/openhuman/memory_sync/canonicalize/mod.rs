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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::openhuman::memory_store::chunks::types::{Metadata, SourceRef};

/// Deserialise a `DateTime<Utc>` from either:
/// - a JSON integer = epoch **milliseconds** (legacy callers — back-compat),
/// - a JSON string = RFC 3339 / ISO-8601 (e.g. `"2026-05-17T19:30:00Z"`), or
///   a decimal string containing epoch milliseconds.
///
/// On an unparseable string a serde error is returned (no silent default).
/// Shared across chat, email, and document canonicalisers.
pub(crate) fn deserialize_flexible_timestamp<'de, D>(
    deserializer: D,
) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawTs {
        Millis(i64),
        Text(String),
    }

    let raw = RawTs::deserialize(deserializer)?;
    match raw {
        RawTs::Millis(ms) => {
            tracing::debug!("[memory][canonicalize] parsed timestamp as epoch-ms: {ms}");
            chrono::TimeZone::timestamp_millis_opt(&Utc, ms)
                .single()
                .ok_or_else(|| serde::de::Error::custom(format!("invalid epoch-ms: {ms}")))
        }
        RawTs::Text(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                tracing::debug!("[memory][canonicalize] parsed timestamp as ISO-8601 string: {s}");
                return Ok(dt.with_timezone(&Utc));
            }
            if let Ok(ms) = s.parse::<i64>() {
                tracing::debug!(
                    "[memory][canonicalize] parsed timestamp as numeric-string epoch-ms: {s}"
                );
                return chrono::TimeZone::timestamp_millis_opt(&Utc, ms)
                    .single()
                    .ok_or_else(|| {
                        serde::de::Error::custom(format!("invalid epoch-ms string: {s}"))
                    });
            }
            Err(serde::de::Error::custom(format!(
                "cannot parse '{s}' as RFC 3339 or epoch-ms"
            )))
        }
    }
}

/// Output of a canonicaliser — one per logical source record
/// (a chat batch, an email, a document).
#[derive(Clone, Debug)]
pub struct CanonicalisedSource {
    /// Canonical Markdown blob produced by the adapter.
    pub markdown: String,
    /// Provenance the chunker will clone onto each emitted [`Chunk`].
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
