//! Typed failure + degradation model for the memory pipeline.
//!
//! The chunk→wiki pipeline and the time-tree summarizer fail in several
//! distinct ways (budget exhausted, missing/invalid key, missing local
//! model, dimension mismatch, extraction timeout, transient network).
//! Historically these all collapsed into an opaque error string and were
//! retried identically — so a hard "Insufficient budget" 4xx burned the
//! retry budget and the user saw a generic `error: N failed jobs`.
//!
//! This module is the single source of truth that fixes that:
//!
//! - [`FailureCode`] enumerates every distinguishable cause.
//! - Each code maps to a [`FailureClass`] (`Transient` ⇒ retry with
//!   backoff, `Unrecoverable` ⇒ fail fast) and a stable i18n
//!   `remediation_key` so the status surface / doctor / job row all show
//!   consistent, actionable text. Embeddings remediation leads with the
//!   local-Ollama path (the steered primary fix), with BYO key secondary.
//! - [`PipelineFailure`] is a `std::error::Error`, so it can be wrapped in
//!   `anyhow` and propagated up through the job processor, then downcast in
//!   the queue worker to decide retry-vs-fail.
//! - [`DegradedState`] captures "the pipeline ran but recall/structure is
//!   reduced" — surfaced so degraded output is never presented as success.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Whether a failure should be retried (`Transient`) or fail fast
/// (`Unrecoverable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Retry with backoff up to `max_attempts` (network 5xx, timeouts,
    /// truncated streams).
    Transient,
    /// Stop immediately — retrying the same input cannot succeed (budget
    /// exhausted, bad/missing key, missing local model, dim mismatch).
    Unrecoverable,
}

impl FailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Unrecoverable => "unrecoverable",
        }
    }
}

/// A distinguishable pipeline failure cause. Each variant carries a fixed
/// [`FailureClass`] and i18n remediation key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// Managed embeddings route returned an out-of-budget error (4xx).
    BudgetExhausted,
    /// No auth/session available for the embeddings provider.
    AuthMissing,
    /// Auth present but rejected (expired/invalid key or JWT).
    AuthInvalid,
    /// No embeddings provider is configured at all.
    EmbeddingsUnconfigured,
    /// Provider returned vectors of an unexpected dimensionality.
    EmbeddingDimMismatch,
    /// A required local model (Ollama) is not available.
    LocalModelUnavailable,
    /// The extraction model timed out / exhausted retries.
    ExtractionTimeout,
    /// Catch-all transient failure (network 5xx, timeout, truncated JSON).
    Transient,
}

impl FailureCode {
    /// Stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budget_exhausted",
            Self::AuthMissing => "auth_missing",
            Self::AuthInvalid => "auth_invalid",
            Self::EmbeddingsUnconfigured => "embeddings_unconfigured",
            Self::EmbeddingDimMismatch => "embedding_dim_mismatch",
            Self::LocalModelUnavailable => "local_model_unavailable",
            Self::ExtractionTimeout => "extraction_timeout",
            Self::Transient => "transient",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "budget_exhausted" => Self::BudgetExhausted,
            "auth_missing" => Self::AuthMissing,
            "auth_invalid" => Self::AuthInvalid,
            "embeddings_unconfigured" => Self::EmbeddingsUnconfigured,
            "embedding_dim_mismatch" => Self::EmbeddingDimMismatch,
            "local_model_unavailable" => Self::LocalModelUnavailable,
            "extraction_timeout" => Self::ExtractionTimeout,
            "transient" => Self::Transient,
            _ => return None,
        })
    }

    /// Retry policy for this cause.
    pub fn class(self) -> FailureClass {
        match self {
            Self::Transient => FailureClass::Transient,
            _ => FailureClass::Unrecoverable,
        }
    }

    /// i18n key for the user-facing remediation. Embeddings causes lead
    /// with the local-Ollama path (the steered primary fix per spec FR-015).
    pub fn remediation_key(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "memory.health.remediation.budget_exhausted",
            Self::AuthMissing => "memory.health.remediation.auth_missing",
            Self::AuthInvalid => "memory.health.remediation.auth_invalid",
            Self::EmbeddingsUnconfigured => "memory.health.remediation.embeddings_unconfigured",
            Self::EmbeddingDimMismatch => "memory.health.remediation.embedding_dim_mismatch",
            Self::LocalModelUnavailable => "memory.health.remediation.local_model_unavailable",
            Self::ExtractionTimeout => "memory.health.remediation.extraction_timeout",
            Self::Transient => "memory.health.remediation.transient",
        }
    }
}

/// A typed pipeline failure: a [`FailureCode`] plus the derived class +
/// remediation key (carried on the wire so the frontend stays
/// presentational) and an optional human-readable detail for logs/diagnosis.
///
/// Implements [`std::error::Error`] so it can be `anyhow`-wrapped at the
/// embed/extract/summarize boundary, propagated through the job processor,
/// and downcast in the queue worker to drive retry-vs-fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineFailure {
    pub code: FailureCode,
    pub class: FailureClass,
    /// i18n key — the frontend resolves this to localized remediation text.
    pub remediation_key: String,
    /// Optional non-localized detail for logs/diagnosis (never a secret).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl PipelineFailure {
    /// Build a failure from a code, deriving class + remediation key.
    pub fn new(code: FailureCode) -> Self {
        Self {
            code,
            class: code.class(),
            remediation_key: code.remediation_key().to_string(),
            detail: None,
        }
    }

    /// Attach a non-localized detail string (truncated by callers; never
    /// log secrets).
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// True when this failure should fail fast (no retry budget).
    pub fn is_unrecoverable(&self) -> bool {
        self.class == FailureClass::Unrecoverable
    }
}

/// Classify an embedding-stage error into a typed [`PipelineFailure`].
///
/// The embed path bottoms out in `embeddings::openai::OpenAiEmbedding::embed`,
/// which on a non-2xx response bails with the message
/// `"Embedding API error (<status>): <body>"` (status is reqwest's
/// `StatusCode` Display, e.g. `402 Payment Required`). Dimension mismatches
/// surface from the memory-tree `CloudEmbedder`/trait validator as
/// `"... returned N dims, expected M"` or `"... dims, expected ..."`. We
/// parse those shapes to decide retry-vs-fail:
///
/// - `401` / `403` → `auth_invalid` (a bearer was sent but rejected).
/// - `402` / `429` / a body mentioning budget/quota/insufficient →
///   `budget_exhausted` (the managed Voyage route is out of budget; the
///   user must bring their own key or top up — retrying won't help).
/// - dimension-mismatch text → `embedding_dim_mismatch`.
/// - everything else (5xx, timeouts, transport, unparseable) → `transient`,
///   so the worker's existing retry-with-backoff still applies.
///
/// Operates on the flattened `anyhow` chain (`{err:#}`) so it still matches
/// when the embed error has been `.context()`-wrapped on the way up.
pub fn classify_embed_error(err: &anyhow::Error) -> PipelineFailure {
    let msg = format!("{err:#}");
    classify_embed_error_str(&msg)
}

/// String-level core of [`classify_embed_error`], split out so unit tests can
/// exercise the mapping without constructing reqwest errors.
pub fn classify_embed_error_str(msg: &str) -> PipelineFailure {
    let lower = msg.to_ascii_lowercase();

    // Dimension mismatch — the trait validator / CloudEmbedder rejects a
    // vector whose length isn't EMBEDDING_DIM. Check before status parsing:
    // it's a 2xx-but-wrong-shape case with no HTTP status to match.
    if lower.contains("dims, expected") || lower.contains("dimensions, expected") {
        return PipelineFailure::new(FailureCode::EmbeddingDimMismatch)
            .with_detail(truncate_detail(msg));
    }

    // Budget/quota wording wins regardless of the numeric status — the
    // managed backend may surface budget exhaustion as 4xx with an explicit
    // body, and we always want the BYO-key remediation here.
    if lower.contains("insufficient budget")
        || lower.contains("budget")
        || lower.contains("quota")
        || lower.contains("payment required")
    {
        return PipelineFailure::new(FailureCode::BudgetExhausted).with_detail(truncate_detail(msg));
    }

    // Parse the HTTP status out of the `Embedding API error (<status>): ...`
    // shape. reqwest renders e.g. `402 Payment Required`, so the first
    // 3-digit run after the opening paren is the code.
    if let Some(code) = parse_http_status(msg) {
        return match code {
            401 | 403 => {
                PipelineFailure::new(FailureCode::AuthInvalid).with_detail(truncate_detail(msg))
            }
            402 | 429 => {
                PipelineFailure::new(FailureCode::BudgetExhausted).with_detail(truncate_detail(msg))
            }
            // 4xx other than the above is a hard client error retrying won't
            // fix (malformed request, model not found); fail fast but tag it
            // generically as auth_invalid's sibling — use Transient only for
            // 5xx/unknown. We treat unknown 4xx as unrecoverable via
            // budget? No — be conservative: only the known codes above are
            // unrecoverable; other 4xx fall through to transient so we don't
            // wedge on a transient 408/425.
            500..=599 => PipelineFailure::new(FailureCode::Transient).with_detail(truncate_detail(msg)),
            _ => PipelineFailure::new(FailureCode::Transient).with_detail(truncate_detail(msg)),
        };
    }

    // No recognizable status — transport error, timeout, connection reset,
    // or an unparseable message. Treat as transient so retry/backoff applies.
    PipelineFailure::new(FailureCode::Transient).with_detail(truncate_detail(msg))
}

/// Extract the first HTTP status code from an `Embedding API error (<status>)`
/// message. Returns the leading 3-digit number inside the first parenthesised
/// group, if present.
fn parse_http_status(msg: &str) -> Option<u16> {
    let open = msg.find('(')?;
    let rest = &msg[open + 1..];
    let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 3 {
        digits.parse().ok()
    } else {
        None
    }
}

/// Cap a detail string so we never balloon logs / wire payloads with a full
/// provider response body. Never contains a secret (it's an error body), but
/// keep it short anyway.
fn truncate_detail(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX).collect();
    format!("{truncated}…")
}

impl fmt::Display for PipelineFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.code.as_str(), self.class.as_str())?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PipelineFailure {}

/// "The pipeline ran, but output quality is reduced." Surfaced so degraded
/// results are never presented as success.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegradedState {
    /// True when embeddings were skipped (no usable provider) so semantic
    /// recall falls back to recency-only.
    pub semantic_recall: bool,
    /// True when extraction yielded empty across the board so the wiki has
    /// no entity/topic structure.
    pub structure: bool,
    /// The cause of the most significant degradation, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<PipelineFailure>,
}

impl DegradedState {
    /// True when any degradation is present.
    pub fn is_degraded(&self) -> bool {
        self.semantic_recall || self.structure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CODES: [FailureCode; 8] = [
        FailureCode::BudgetExhausted,
        FailureCode::AuthMissing,
        FailureCode::AuthInvalid,
        FailureCode::EmbeddingsUnconfigured,
        FailureCode::EmbeddingDimMismatch,
        FailureCode::LocalModelUnavailable,
        FailureCode::ExtractionTimeout,
        FailureCode::Transient,
    ];

    #[test]
    fn every_code_has_class_and_nonempty_remediation_key() {
        for code in ALL_CODES {
            let key = code.remediation_key();
            assert!(!key.is_empty(), "{} has empty remediation key", code.as_str());
            assert!(
                key.starts_with("memory.health.remediation."),
                "{} remediation key has unexpected prefix: {key}",
                code.as_str()
            );
            // class() must be total (no panic) and only Transient is transient.
            let class = code.class();
            if code == FailureCode::Transient {
                assert_eq!(class, FailureClass::Transient);
            } else {
                assert_eq!(
                    class,
                    FailureClass::Unrecoverable,
                    "{} should be unrecoverable",
                    code.as_str()
                );
            }
        }
    }

    #[test]
    fn code_str_roundtrips() {
        for code in ALL_CODES {
            assert_eq!(FailureCode::from_str(code.as_str()), Some(code));
        }
        assert_eq!(FailureCode::from_str("nonsense"), None);
    }

    #[test]
    fn new_fills_class_and_remediation_from_code() {
        let f = PipelineFailure::new(FailureCode::BudgetExhausted);
        assert_eq!(f.code, FailureCode::BudgetExhausted);
        assert_eq!(f.class, FailureClass::Unrecoverable);
        assert_eq!(f.remediation_key, "memory.health.remediation.budget_exhausted");
        assert!(f.detail.is_none());
        assert!(f.is_unrecoverable());
    }

    #[test]
    fn with_detail_and_display() {
        let f = PipelineFailure::new(FailureCode::Transient).with_detail("HTTP 503");
        assert_eq!(f.detail.as_deref(), Some("HTTP 503"));
        assert!(!f.is_unrecoverable());
        assert_eq!(f.to_string(), "transient (transient): HTTP 503");
    }

    #[test]
    fn pipeline_failure_serde_roundtrips() {
        let f = PipelineFailure::new(FailureCode::EmbeddingDimMismatch).with_detail("got 3072");
        let json = serde_json::to_string(&f).unwrap();
        let back: PipelineFailure = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
        // detail omitted when None.
        let none = PipelineFailure::new(FailureCode::AuthMissing);
        assert!(!serde_json::to_string(&none).unwrap().contains("detail"));
    }

    #[test]
    fn degraded_state_default_is_healthy() {
        let d = DegradedState::default();
        assert!(!d.is_degraded());
        let d2 = DegradedState {
            structure: true,
            ..Default::default()
        };
        assert!(d2.is_degraded());
    }

    #[test]
    fn pipeline_failure_is_error_and_downcasts_from_anyhow() {
        let err: anyhow::Error =
            anyhow::Error::new(PipelineFailure::new(FailureCode::BudgetExhausted));
        let downcast = err.downcast_ref::<PipelineFailure>();
        assert!(downcast.is_some());
        assert!(downcast.unwrap().is_unrecoverable());
    }

    // ── classify_embed_error (T008) ──────────────────────────────────────

    #[test]
    fn classify_budget_from_body_wording() {
        // The managed Voyage route surfaces budget exhaustion in the body.
        let f = classify_embed_error_str(
            "Embedding API error (400 Bad Request): {\"error\":\"Insufficient budget\"}",
        );
        assert_eq!(f.code, FailureCode::BudgetExhausted);
        assert!(f.is_unrecoverable());
    }

    #[test]
    fn classify_budget_from_402_and_429() {
        for status in ["402 Payment Required", "429 Too Many Requests"] {
            let f = classify_embed_error_str(&format!("Embedding API error ({status}): nope"));
            assert_eq!(
                f.code,
                FailureCode::BudgetExhausted,
                "status {status} should map to budget_exhausted"
            );
        }
    }

    #[test]
    fn classify_auth_from_401_403() {
        for status in ["401 Unauthorized", "403 Forbidden"] {
            let f = classify_embed_error_str(&format!("Embedding API error ({status}): denied"));
            assert_eq!(f.code, FailureCode::AuthInvalid, "status {status}");
            assert!(f.is_unrecoverable());
        }
    }

    #[test]
    fn classify_dim_mismatch() {
        let f = classify_embed_error_str("cloud embedder returned 3072 dims, expected 1024");
        assert_eq!(f.code, FailureCode::EmbeddingDimMismatch);
        assert!(f.is_unrecoverable());
    }

    #[test]
    fn classify_5xx_is_transient() {
        let f = classify_embed_error_str("Embedding API error (503 Service Unavailable): retry");
        assert_eq!(f.code, FailureCode::Transient);
        assert!(!f.is_unrecoverable());
    }

    #[test]
    fn classify_transport_error_is_transient() {
        let f = classify_embed_error_str("error sending request for url (...): connection reset");
        assert_eq!(f.code, FailureCode::Transient);
        assert!(!f.is_unrecoverable());
    }

    #[test]
    fn classify_through_anyhow_context_chain() {
        // The embed error is commonly `.context()`-wrapped on the way up;
        // the flattened `{err:#}` must still classify.
        let base = anyhow::anyhow!("Embedding API error (402 Payment Required): out of budget");
        let wrapped = base.context("cloud embeddings failed").context("seal embed");
        let f = classify_embed_error(&wrapped);
        assert_eq!(f.code, FailureCode::BudgetExhausted);
    }

    #[test]
    fn parse_http_status_extracts_leading_code() {
        assert_eq!(
            parse_http_status("Embedding API error (402 Payment Required): x"),
            Some(402)
        );
        assert_eq!(parse_http_status("no parens here"), None);
        assert_eq!(parse_http_status("(not a status): x"), None);
    }

    #[test]
    fn truncate_detail_caps_length() {
        let long = "x".repeat(500);
        let out = truncate_detail(&long);
        assert!(out.chars().count() <= 201, "got {}", out.chars().count());
        assert!(out.ends_with('…'));
    }
}
