//! Minimum model requirements the memory layer imposes on local models.
//!
//! The memory tree's embedder (`bge-m3`) is requested with
//! `num_ctx = 8192` (see
//! [`crate::openhuman::memory::score::embed::ollama::EMBED_NUM_CTX`])
//! and the summariser hard-caps its output to fit that 8192-token embed
//! ceiling. A local model whose native context window is below this floor
//! silently truncates chunks/summaries and corrupts recall, so we refuse
//! to accept any local model reporting a context window smaller than
//! [`MIN_CONTEXT_TOKENS`]. There is **no upper requirement** — the
//! pipeline is deliberately capped at 8k and never needs more from a
//! local model.

use serde::Serialize;

/// Minimum native context window (tokens) a local model must advertise to
/// be accepted by the memory layer.
///
/// Re-exported from the embedder's own `EMBED_NUM_CTX` so this gate can
/// never drift from what the memory pipeline actually requests at embed
/// time. Changing the embedder's context request automatically moves the
/// acceptance floor with it.
pub const MIN_CONTEXT_TOKENS: u64 =
    crate::openhuman::memory::score::embed::ollama::EMBED_NUM_CTX as u64;

/// Verdict for a single model's context window against
/// [`MIN_CONTEXT_TOKENS`]. Serialized into the diagnostics payload so the
/// frontend can render / reject each installed model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContextEligibility {
    /// Context window ≥ the minimum. Accepted.
    Ok { context_length: u64 },
    /// Context window below the minimum. Rejected for memory-layer use.
    BelowMinimum { context_length: u64, required: u64 },
    /// Context window could not be determined (`/api/show` error or the
    /// metadata key is absent). Not hard-rejected — surfaced as unknown so
    /// the UI can warn without blocking a model that may still be fine.
    Unknown { required: u64 },
}

impl ContextEligibility {
    /// `true` only when the model is positively accepted.
    pub fn is_accepted(&self) -> bool {
        matches!(self, ContextEligibility::Ok { .. })
    }

    /// `true` when the model is conclusively rejected (reported a context
    /// window below the floor). `Unknown` is **not** a rejection.
    pub fn is_rejected(&self) -> bool {
        matches!(self, ContextEligibility::BelowMinimum { .. })
    }
}

/// Classify a model's (optional) reported context length against the
/// memory-layer minimum.
pub fn evaluate_context(context_length: Option<u64>) -> ContextEligibility {
    match context_length {
        Some(ctx) if ctx >= MIN_CONTEXT_TOKENS => ContextEligibility::Ok {
            context_length: ctx,
        },
        Some(ctx) => ContextEligibility::BelowMinimum {
            context_length: ctx,
            required: MIN_CONTEXT_TOKENS,
        },
        None => ContextEligibility::Unknown {
            required: MIN_CONTEXT_TOKENS,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_context_tracks_embedder_request() {
        // The acceptance floor must equal what the memory embedder actually
        // requests; this guards against the two drifting apart.
        assert_eq!(
            MIN_CONTEXT_TOKENS,
            crate::openhuman::memory::score::embed::ollama::EMBED_NUM_CTX as u64
        );
        assert_eq!(MIN_CONTEXT_TOKENS, 8_192);
    }

    #[test]
    fn at_or_above_minimum_is_accepted() {
        let exact = evaluate_context(Some(8_192));
        assert!(exact.is_accepted());
        assert_eq!(
            exact,
            ContextEligibility::Ok {
                context_length: 8_192
            }
        );

        let above = evaluate_context(Some(32_768));
        assert!(above.is_accepted());
        assert!(!above.is_rejected());
    }

    #[test]
    fn below_minimum_is_rejected_with_required_floor() {
        let verdict = evaluate_context(Some(2_048));
        assert!(verdict.is_rejected());
        assert!(!verdict.is_accepted());
        assert_eq!(
            verdict,
            ContextEligibility::BelowMinimum {
                context_length: 2_048,
                required: 8_192,
            }
        );
    }

    #[test]
    fn unknown_context_is_neither_accepted_nor_rejected() {
        let verdict = evaluate_context(None);
        assert!(!verdict.is_accepted());
        assert!(!verdict.is_rejected());
        assert_eq!(verdict, ContextEligibility::Unknown { required: 8_192 });
    }

    #[test]
    fn eligibility_serializes_tagged() {
        let json = serde_json::to_value(evaluate_context(Some(4_096))).unwrap();
        assert_eq!(json["status"], "below_minimum");
        assert_eq!(json["context_length"], 4_096);
        assert_eq!(json["required"], 8_192);

        let ok = serde_json::to_value(evaluate_context(Some(8_192))).unwrap();
        assert_eq!(ok["status"], "ok");
        assert_eq!(ok["context_length"], 8_192);

        let unknown = serde_json::to_value(evaluate_context(None)).unwrap();
        assert_eq!(unknown["status"], "unknown");
        assert_eq!(unknown["required"], 8_192);
    }
}
