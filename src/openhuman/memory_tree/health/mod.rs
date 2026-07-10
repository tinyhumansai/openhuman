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

pub mod doctor;
pub use doctor::{async_run_doctor, run_doctor, DoctorCounters, DoctorReport, StageHealth};

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
    /// No summarization provider could be resolved for "Build Summary Trees"
    /// — neither local AI nor a configured cloud chat provider. Distinct from
    /// [`LocalModelUnavailable`](Self::LocalModelUnavailable), which implies the
    /// local path was selected; this covers the cloud-only setup whose provider
    /// failed to resolve, so the remediation names both paths.
    SummarizerUnavailable,
    /// The embedding provider refused an empty/whitespace input at the
    /// pre-flight guard (#13021). Unrecoverable per-row: the offending row
    /// will never become embeddable, so the worker must tombstone it instead
    /// of retrying. Bail wording for both `OpenAiEmbedding::embed` and
    /// `OpenHumanCloudEmbedding::embed` starts with
    /// `"<name> embed: refusing empty/whitespace input ..."`.
    EmptyInputRefused,
    /// The host filesystem cannot service the memory_tree path — `create_dir`
    /// / DB open returned a persistent OS-level I/O error (EIO `5`, ENOSPC
    /// `28`, EROFS `30`), e.g. a failing/disconnected SD card or a volume the
    /// kernel remounted read-only. Unrecoverable from inside the app: only the
    /// user can reseat/replace/free the storage. Distinct from the embeddings
    /// provider faults above and from the SQLite-level `SQLITE_FULL` /
    /// `SQLITE_CORRUPT` handled in the queue worker — this is the
    /// directory/DB-init layer below them.
    StorageUnavailable,
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
            Self::SummarizerUnavailable => "summarizer_unavailable",
            Self::EmptyInputRefused => "empty_input_refused",
            Self::StorageUnavailable => "storage_unavailable",
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
            "summarizer_unavailable" => Self::SummarizerUnavailable,
            "empty_input_refused" => Self::EmptyInputRefused,
            "storage_unavailable" => Self::StorageUnavailable,
            "transient" => Self::Transient,
            _ => return None,
        })
    }

    /// Retry policy for this cause.
    pub fn class(self) -> FailureClass {
        match self {
            Self::Transient | Self::ExtractionTimeout => FailureClass::Transient,
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
            Self::SummarizerUnavailable => "memory.health.remediation.summarizer_unavailable",
            Self::EmptyInputRefused => "memory.health.remediation.empty_input_refused",
            Self::StorageUnavailable => "memory.health.remediation.storage_unavailable",
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

    // #13021: client-side refusal from the provider pre-flight guard fires
    // *before* any HTTP round-trip, so it carries no `Embedding API error
    // (<status>)` shape. Without an explicit match it would fall through to
    // `Transient` and the `reembed_backfill` worker would retry the same
    // un-embeddable row forever (and eventually fail the whole job).
    // Classify as unrecoverable per-row so the worker tombstones the chunk /
    // summary instead. Both `OpenAiEmbedding::embed` and
    // `OpenHumanCloudEmbedding::embed` use the literal phrase
    // "refusing empty/whitespace".
    if lower.contains("refusing empty/whitespace") {
        return PipelineFailure::new(FailureCode::EmptyInputRefused)
            .with_detail(truncate_detail(msg));
    }

    // Sibling of the #13021 case above: `OpenHumanCloudEmbedding::resolve_bearer`
    // bails *before any HTTP round-trip* when the desktop/backend session
    // bearer is absent (user signed out), with the literal phrase
    // "No backend session for cloud embeddings ..." (see
    // `src/openhuman/embeddings/cloud.rs`). Being a client-side bail it carries
    // no `Embedding API error (<status>)` shape, so without this match it falls
    // through to `Transient` — the Memory Tree then shows "temporary error…
    // will retry automatically" and the worker retries an auth failure that a
    // retry can never fix. Classify as `AuthMissing` so the health banner
    // surfaces the "log in to OpenHuman" remediation and the job fails fast.
    if lower.contains("no backend session") {
        return PipelineFailure::new(FailureCode::AuthMissing).with_detail(truncate_detail(msg));
    }

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
        return PipelineFailure::new(FailureCode::BudgetExhausted)
            .with_detail(truncate_detail(msg));
    }

    // Parse the HTTP status out of the `Embedding API error (<status>): ...`
    // shape. reqwest renders e.g. `402 Payment Required`, so the first
    // 3-digit run after the opening paren is the code.
    if let Some(code) = parse_http_status(msg) {
        return match code {
            401 | 403 => {
                PipelineFailure::new(FailureCode::AuthInvalid).with_detail(truncate_detail(msg))
            }
            402 => {
                PipelineFailure::new(FailureCode::BudgetExhausted).with_detail(truncate_detail(msg))
            }
            429 => PipelineFailure::new(FailureCode::Transient).with_detail(truncate_detail(msg)),
            // 4xx other than the above is a hard client error retrying won't
            // fix (malformed request, model not found); fail fast but tag it
            // generically as auth_invalid's sibling — use Transient only for
            // 5xx/unknown. We treat unknown 4xx as unrecoverable via
            // budget? No — be conservative: only the known codes above are
            // unrecoverable; other 4xx fall through to transient so we don't
            // wedge on a transient 408/425.
            500..=599 => {
                PipelineFailure::new(FailureCode::Transient).with_detail(truncate_detail(msg))
            }
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
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
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
    /// True when the memory_tree's own storage path is unusable — the host
    /// filesystem returned a persistent I/O error on dir-create / DB open
    /// (EIO/ENOSPC/EROFS). This is the most severe degradation: the pipeline
    /// can't even open its DB, so nothing else runs. `#[serde(default)]` keeps
    /// the wire format backward-compatible (older clients omit it → `false`).
    #[serde(default)]
    pub storage: bool,
    /// The cause of the most significant degradation, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<PipelineFailure>,
}

impl DegradedState {
    /// True when any degradation is present.
    pub fn is_degraded(&self) -> bool {
        self.semantic_recall || self.structure || self.storage
    }
}

// ── Process-visible degradation flags ────────────────────────────────────
//
// The embed/extract stages run deep inside the job worker, far from the
// `pipeline_status` RPC. Rather than thread a `DegradedState` return up
// through every call site, the stages set these process-global atomics when
// they detect a degraded condition (no usable embedder → semantic recall
// disabled; extraction empty across the board → no structure). The status /
// doctor surface reads them via [`current_degraded_state`]. They reflect the
// most recent run, are cheap, and never block — a coarse "is recall/structure
// currently degraded?" signal, intentionally not per-namespace.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static SEMANTIC_RECALL_DEGRADED: AtomicBool = AtomicBool::new(false);
static STRUCTURE_DEGRADED: AtomicBool = AtomicBool::new(false);
/// The host filesystem can't service the memory_tree path (EIO/ENOSPC/EROFS).
/// Set by the queue worker's host-I/O arm; cleared on the next successful
/// claim (storage recovered). Most severe — outranks recall/structure.
static STORAGE_DEGRADED: AtomicBool = AtomicBool::new(false);
/// Per-flag degradation cause as a `FailureCode` discriminant (0 = none).
/// Tracked separately per flag so clearing one degradation can't leave the
/// other reporting a stale cause (e.g. mark recall, mark structure, clear
/// structure → recall must still report its OWN cause, not structure's).
static SEMANTIC_RECALL_CAUSE: AtomicU8 = AtomicU8::new(0);
static STRUCTURE_CAUSE: AtomicU8 = AtomicU8::new(0);
static STORAGE_CAUSE: AtomicU8 = AtomicU8::new(0);

fn code_to_u8(code: FailureCode) -> u8 {
    match code {
        FailureCode::BudgetExhausted => 1,
        FailureCode::AuthMissing => 2,
        FailureCode::AuthInvalid => 3,
        FailureCode::EmbeddingsUnconfigured => 4,
        FailureCode::EmbeddingDimMismatch => 5,
        FailureCode::LocalModelUnavailable => 6,
        FailureCode::ExtractionTimeout => 7,
        FailureCode::SummarizerUnavailable => 8,
        FailureCode::Transient => 9,
        FailureCode::EmptyInputRefused => 10,
        FailureCode::StorageUnavailable => 11,
    }
}

fn u8_to_code(v: u8) -> Option<FailureCode> {
    Some(match v {
        1 => FailureCode::BudgetExhausted,
        2 => FailureCode::AuthMissing,
        3 => FailureCode::AuthInvalid,
        4 => FailureCode::EmbeddingsUnconfigured,
        5 => FailureCode::EmbeddingDimMismatch,
        6 => FailureCode::LocalModelUnavailable,
        7 => FailureCode::ExtractionTimeout,
        8 => FailureCode::SummarizerUnavailable,
        9 => FailureCode::Transient,
        10 => FailureCode::EmptyInputRefused,
        11 => FailureCode::StorageUnavailable,
        _ => return None,
    })
}

/// Record that semantic recall is degraded (embeddings were skipped because no
/// usable provider is available). `cause` names why so the status surface can
/// lead the user to the fix. Idempotent / cheap; safe to call per embed-stage.
pub fn mark_semantic_recall_degraded(cause: FailureCode) {
    SEMANTIC_RECALL_DEGRADED.store(true, Ordering::Relaxed);
    SEMANTIC_RECALL_CAUSE.store(code_to_u8(cause), Ordering::Relaxed);
}

/// Clear the semantic-recall degraded flag — call when an embed succeeds, so
/// the surface recovers once the user fixes the provider. Clears only this
/// flag's cause; a still-active structure degradation keeps its own.
pub fn clear_semantic_recall_degraded() {
    SEMANTIC_RECALL_DEGRADED.store(false, Ordering::Relaxed);
    SEMANTIC_RECALL_CAUSE.store(0, Ordering::Relaxed);
}

/// Record that wiki structure is degraded (extraction yielded nothing across
/// the board). `cause` is typically [`FailureCode::ExtractionTimeout`].
pub fn mark_structure_degraded(cause: FailureCode) {
    STRUCTURE_DEGRADED.store(true, Ordering::Relaxed);
    STRUCTURE_CAUSE.store(code_to_u8(cause), Ordering::Relaxed);
}

/// Clear the structure degraded flag — call when extraction yields entities.
/// Clears only this flag's cause.
pub fn clear_structure_degraded() {
    STRUCTURE_DEGRADED.store(false, Ordering::Relaxed);
    STRUCTURE_CAUSE.store(0, Ordering::Relaxed);
}

/// Record that the memory_tree storage path is unusable — the host filesystem
/// returned a persistent I/O error (EIO/ENOSPC/EROFS) on dir-create / DB open.
/// `cause` is typically [`FailureCode::StorageUnavailable`]. Set by the queue
/// worker's host-I/O arm so the status surface tells the user to check their
/// disk; idempotent / cheap.
pub fn mark_storage_degraded(cause: FailureCode) {
    STORAGE_DEGRADED.store(true, Ordering::Relaxed);
    STORAGE_CAUSE.store(code_to_u8(cause), Ordering::Relaxed);
}

/// Clear the storage degraded flag — call when a claim succeeds (the DB opened,
/// so the host filesystem recovered), so the surface self-heals. Clears only
/// this flag's cause.
pub fn clear_storage_degraded() {
    STORAGE_DEGRADED.store(false, Ordering::Relaxed);
    STORAGE_CAUSE.store(0, Ordering::Relaxed);
}

/// Test-only serialization + reset for the process-global degraded flags.
///
/// The flags are a single process-wide signal, so tests across *different*
/// modules (factory, extract::llm, tree::rpc) that set or read them race under
/// cargo's parallel runner. Any such test must `let _g = test_guard();` at the
/// top: it takes a shared mutex (serialising all flag-touching tests) and
/// resets both flags to a clean baseline so the test starts deterministic.
#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let g = LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    SEMANTIC_RECALL_DEGRADED.store(false, Ordering::Relaxed);
    STRUCTURE_DEGRADED.store(false, Ordering::Relaxed);
    STORAGE_DEGRADED.store(false, Ordering::Relaxed);
    SEMANTIC_RECALL_CAUSE.store(0, Ordering::Relaxed);
    STRUCTURE_CAUSE.store(0, Ordering::Relaxed);
    STORAGE_CAUSE.store(0, Ordering::Relaxed);
    g
}

/// Snapshot the current process-global [`DegradedState`] for the status /
/// doctor surface. The `cause` is populated from the last recorded
/// [`FailureCode`] when either flag is set.
pub fn current_degraded_state() -> DegradedState {
    let semantic_recall = SEMANTIC_RECALL_DEGRADED.load(Ordering::Relaxed);
    let structure = STRUCTURE_DEGRADED.load(Ordering::Relaxed);
    let storage = STORAGE_DEGRADED.load(Ordering::Relaxed);
    // Each flag carries its own cause; pick the most actionable one to surface.
    // Storage degradation is reported first — the host FS can't open the DB, so
    // it's the foundational failure beneath both recall and structure (no point
    // telling the user "configure embeddings" when the disk is dying). Then
    // structure (extraction failing → empty wiki), then recall. Either way the
    // cause reflects a CURRENTLY-active flag.
    let cause = if storage {
        u8_to_code(STORAGE_CAUSE.load(Ordering::Relaxed)).map(PipelineFailure::new)
    } else if structure {
        u8_to_code(STRUCTURE_CAUSE.load(Ordering::Relaxed)).map(PipelineFailure::new)
    } else if semantic_recall {
        u8_to_code(SEMANTIC_RECALL_CAUSE.load(Ordering::Relaxed)).map(PipelineFailure::new)
    } else {
        None
    };
    DegradedState {
        semantic_recall,
        structure,
        storage,
        cause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CODES: [FailureCode; 11] = [
        FailureCode::BudgetExhausted,
        FailureCode::AuthMissing,
        FailureCode::AuthInvalid,
        FailureCode::EmbeddingsUnconfigured,
        FailureCode::EmbeddingDimMismatch,
        FailureCode::LocalModelUnavailable,
        FailureCode::ExtractionTimeout,
        FailureCode::SummarizerUnavailable,
        FailureCode::EmptyInputRefused,
        FailureCode::StorageUnavailable,
        FailureCode::Transient,
    ];

    #[test]
    fn every_code_has_class_and_nonempty_remediation_key() {
        for code in ALL_CODES {
            let key = code.remediation_key();
            assert!(
                !key.is_empty(),
                "{} has empty remediation key",
                code.as_str()
            );
            assert!(
                key.starts_with("memory.health.remediation."),
                "{} remediation key has unexpected prefix: {key}",
                code.as_str()
            );
            // class() must be total (no panic); Transient + ExtractionTimeout
            // are retryable, everything else is unrecoverable.
            let class = code.class();
            match code {
                FailureCode::Transient | FailureCode::ExtractionTimeout => {
                    assert_eq!(
                        class,
                        FailureClass::Transient,
                        "{} should be transient",
                        code.as_str()
                    );
                }
                _ => {
                    assert_eq!(
                        class,
                        FailureClass::Unrecoverable,
                        "{} should be unrecoverable",
                        code.as_str()
                    );
                }
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
        assert_eq!(
            f.remediation_key,
            "memory.health.remediation.budget_exhausted"
        );
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
    fn classify_budget_from_402() {
        let f = classify_embed_error_str("Embedding API error (402 Payment Required): nope");
        assert_eq!(f.code, FailureCode::BudgetExhausted);
        assert!(f.is_unrecoverable());
    }

    #[test]
    fn classify_429_rate_limit_as_transient() {
        let f = classify_embed_error_str("Embedding API error (429 Too Many Requests): nope");
        assert_eq!(f.code, FailureCode::Transient);
        assert!(!f.is_unrecoverable());
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

    /// #13021: the provider pre-flight bail wording from both OpenAI and the
    /// cloud wrapper must classify as `EmptyInputRefused` (unrecoverable) so
    /// `reembed_backfill` tombstones the offending row instead of retrying
    /// the same blank input forever and eventually failing the job.
    #[test]
    fn classify_empty_input_refusal_as_unrecoverable() {
        for msg in [
            "openai embed: refusing empty/whitespace input at index 0 of 1 (model=text-embedding-3-small)",
            "cloud embed: refusing empty/whitespace input at index 2 of 5 (model=embedding-v1)",
        ] {
            let f = classify_embed_error_str(msg);
            assert_eq!(
                f.code,
                FailureCode::EmptyInputRefused,
                "expected EmptyInputRefused for {msg:?}"
            );
            assert!(
                f.is_unrecoverable(),
                "EmptyInputRefused must be unrecoverable for {msg:?}"
            );
        }
    }

    /// The refusal must out-rank the dim-mismatch and budget rules even when
    /// the wrapped error happens to contain those tokens — the refusal phrase
    /// is the most specific signal and the only one that means "this row is
    /// permanently un-embeddable", not "the provider is misbehaving".
    #[test]
    fn classify_empty_input_refusal_through_anyhow_context_chain() {
        let base = anyhow::anyhow!(
            "openai embed: refusing empty/whitespace input at index 0 of 1 (model=embedding-v1)"
        );
        let wrapped = base
            .context("embed summary during seal tree_id=t level=0")
            .context("reembed_backfill chunk_id=c");
        let f = classify_embed_error(&wrapped);
        assert_eq!(f.code, FailureCode::EmptyInputRefused);
        assert!(f.is_unrecoverable());
    }

    /// #4359: `OpenHumanCloudEmbedding::resolve_bearer` bails with "No backend
    /// session for cloud embeddings ..." *before any HTTP call* when the user
    /// is signed out. This must classify as `AuthMissing` (unrecoverable, "log
    /// in to OpenHuman" remediation) rather than falling through to `Transient`
    /// ("will retry automatically" — a loop that an auth failure can never win).
    #[test]
    fn classify_no_backend_session_as_auth_missing() {
        let msg = "No backend session for cloud embeddings: log in to OpenHuman, or set \
                   memory.embedding_provider to \"ollama\" / \"none\" in config.toml";
        let f = classify_embed_error_str(msg);
        assert_eq!(
            f.code,
            FailureCode::AuthMissing,
            "expected AuthMissing for {msg:?}"
        );
        assert_eq!(f.class, FailureClass::Unrecoverable);
        assert_eq!(f.remediation_key, "memory.health.remediation.auth_missing");
        assert!(f.is_unrecoverable());
    }

    /// The match must be case-insensitive and survive `anyhow` context wrapping:
    /// the bail is `.context()`-wrapped on its way up through the embed pipeline
    /// (e.g. `embed_each_via_provider` adds "cloud embeddings failed"), and
    /// `classify_embed_error` flattens the chain via `{err:#}`.
    #[test]
    fn classify_no_backend_session_through_anyhow_context_chain() {
        let base = anyhow::anyhow!(
            "No backend session for cloud embeddings: log in to OpenHuman, or set \
             memory.embedding_provider to \"ollama\" / \"none\" in config.toml"
        );
        let wrapped = base
            .context("cloud embeddings failed")
            .context("reembed_backfill chunk_id=c");
        let f = classify_embed_error(&wrapped);
        assert_eq!(f.code, FailureCode::AuthMissing);
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
        let wrapped = base
            .context("cloud embeddings failed")
            .context("seal embed");
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

    /// Regression (CodeRabbit): per-flag causes. Mark recall, then structure,
    /// then clear structure — recall must still report its OWN cause, not the
    /// (now-cleared) structure cause. With the old single shared slot this
    /// surfaced the wrong remediation.
    #[test]
    fn degraded_cause_is_per_flag_not_shared() {
        let _g = test_guard(); // resets both flags + causes

        // Recall degraded for embeddings reason; structure degraded for extraction.
        mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
        mark_structure_degraded(FailureCode::ExtractionTimeout);

        // Structure takes precedence while both are active.
        let s = current_degraded_state();
        assert!(s.semantic_recall && s.structure);
        assert_eq!(
            s.cause.as_ref().map(|c| c.code),
            Some(FailureCode::ExtractionTimeout)
        );

        // Clear structure — recall stays, and its cause must be the RECALL one,
        // not the cleared structure cause.
        clear_structure_degraded();
        let s = current_degraded_state();
        assert!(s.semantic_recall && !s.structure);
        assert_eq!(
            s.cause.as_ref().map(|c| c.code),
            Some(FailureCode::EmbeddingsUnconfigured),
            "recall must keep its own cause after structure clears"
        );

        // Clear recall too — fully healthy, no cause.
        clear_semantic_recall_degraded();
        let s = current_degraded_state();
        assert!(!s.is_degraded());
        assert!(s.cause.is_none());
    }

    /// `StorageUnavailable` is the foundational host-FS failure: unrecoverable,
    /// with its own remediation key.
    #[test]
    fn storage_unavailable_is_unrecoverable_with_key() {
        let f = PipelineFailure::new(FailureCode::StorageUnavailable);
        assert_eq!(f.class, FailureClass::Unrecoverable);
        assert!(f.is_unrecoverable());
        assert_eq!(
            f.remediation_key,
            "memory.health.remediation.storage_unavailable"
        );
        // discriminant round-trips through the per-flag u8 mapping.
        assert_eq!(
            u8_to_code(code_to_u8(FailureCode::StorageUnavailable)),
            Some(FailureCode::StorageUnavailable)
        );
    }

    /// Storage degradation outranks both structure and recall in
    /// `current_degraded_state` — the host can't open the DB, so the disk fix
    /// is the one actionable thing to surface. Clearing storage falls back to
    /// the next-most-severe active cause (structure), each keeping its own.
    #[test]
    fn storage_degradation_outranks_structure_and_recall() {
        let _g = test_guard(); // resets all flags + causes

        mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
        mark_structure_degraded(FailureCode::ExtractionTimeout);
        mark_storage_degraded(FailureCode::StorageUnavailable);

        // All three active → storage wins.
        let s = current_degraded_state();
        assert!(s.storage && s.structure && s.semantic_recall);
        assert!(s.is_degraded());
        assert_eq!(
            s.cause.as_ref().map(|c| c.code),
            Some(FailureCode::StorageUnavailable)
        );

        // Clear storage → structure becomes the surfaced cause (its OWN, not
        // storage's stale one).
        clear_storage_degraded();
        let s = current_degraded_state();
        assert!(!s.storage && s.structure);
        assert_eq!(
            s.cause.as_ref().map(|c| c.code),
            Some(FailureCode::ExtractionTimeout)
        );
    }
}
