//! Channel preference producer — Phase 2.
//!
//! Emits a single [`FacetClass::Channel`] Structural candidate for the
//! session's primary communication channel (e.g. `desktop-chat`, `web_chat`).
//! Called once at agent construction when learning is enabled so the stability
//! detector can promote a durable `channel/primary` facet over time.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::openhuman::agent::learning::candidate::{
    self, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};

/// Initial confidence for a structural channel signal from the live session.
const CONF_CHANNEL: f64 = 0.85;

/// Normalise a raw event-channel id into a stable facet value.
///
/// Empty / whitespace-only inputs are rejected. Values are trimmed and
/// lowercased so `Web_Chat` and `web_chat` collapse to the same facet value.
pub fn normalize_channel_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("internal") {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// Push a `channel/primary=<id>` Structural candidate into the global buffer.
///
/// No-ops when `channel` is empty or the reserved `internal` id used by
/// standalone/test builders. Returns `true` when a candidate was pushed.
pub fn emit_primary_channel(channel: &str) -> bool {
    let Some(value) = normalize_channel_id(channel) else {
        tracing::debug!(
            "[learning::extract::channel] skip emit: channel={channel:?} (empty or internal)"
        );
        return false;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let candidate = LearningCandidate {
        class: FacetClass::Channel,
        key: "primary".to_string(),
        value: value.clone(),
        cue_family: CueFamily::Structural,
        evidence: EvidenceRef::Episodic {
            // Synthetic id: producers without an episodic_log row use a
            // placeholder; the stability detector still aggregates by
            // (class, key).
            episodic_id: 0,
        },
        initial_confidence: CONF_CHANNEL,
        observed_at: now,
    };

    candidate::global().push(candidate);
    tracing::debug!(
        "[learning::extract::channel] emitted channel/primary={value} (Structural)"
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::learning::candidate::{self, FacetClass};

    #[test]
    fn normalize_rejects_empty_and_internal() {
        assert_eq!(normalize_channel_id(""), None);
        assert_eq!(normalize_channel_id("   "), None);
        assert_eq!(normalize_channel_id("internal"), None);
        assert_eq!(normalize_channel_id("INTERNAL"), None);
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(
            normalize_channel_id("Web_Chat").as_deref(),
            Some("web_chat")
        );
        assert_eq!(
            normalize_channel_id("desktop-chat").as_deref(),
            Some("desktop-chat")
        );
    }

    #[test]
    fn emit_primary_channel_pushes_structural_candidate() {
        // Drain any leftover candidates from other tests sharing the global buffer.
        let _ = candidate::global().drain();
        assert!(emit_primary_channel("desktop-chat"));
        let drained = candidate::global().drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].class, FacetClass::Channel);
        assert_eq!(drained[0].key, "primary");
        assert_eq!(drained[0].value, "desktop-chat");
        assert_eq!(drained[0].cue_family, CueFamily::Structural);
    }

    #[test]
    fn emit_primary_channel_skips_internal() {
        let _ = candidate::global().drain();
        assert!(!emit_primary_channel("internal"));
        assert!(candidate::global().drain().is_empty());
    }
}
