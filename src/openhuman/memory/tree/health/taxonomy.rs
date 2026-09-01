//! The pipeline failure + degradation taxonomy, owned by this host (#5560).
//!
//! `FailureCode`, `FailureClass`, `PipelineFailure` and `DegradedState` are the
//! vocabulary the memory-tree status surfaces speak: which distinguishable
//! thing went wrong, whether retrying can fix it, the i18n key that tells the
//! user what to do, and "the pipeline ran but recall/structure is reduced".
//!
//! # Why they are defined here rather than re-exported
//!
//! They were `tinycortex::memory::health`'s, reached through a `pub use` in
//! [`super`]. That re-export was the **entire** remaining production
//! `tinycortex` surface of the memory tree — deleting the dependency from
//! `Cargo.toml` produced one unresolved-module error, in `health/mod.rs`, and a
//! cascade of unresolved imports of these four names behind it. #5560 sheds
//! that crate, so a type this host serialises into its own RPC responses has to
//! be this host's.
//!
//! No contract door was needed for the move. The *values* already arrive over
//! the bus: `MemoryMaintenance::{degraded_state, diagnose}` answer with
//! `DegradedCapabilities` and `Diagnosis`, whose `code` and `class` are
//! open-vocabulary strings carrying exactly the snake_case spellings below.
//! [`report`](super::report) parses them into these types. What was missing was
//! never a call — it was ownership of the vocabulary the answer is parsed into.
//!
//! # The wire is the constraint, and it is pinned
//!
//! These are field-for-field and spelling-for-spelling what the engine's types
//! serialised to, including `PipelineFailure::detail`'s
//! `skip_serializing_if` and `DegradedState::storage`'s `#[serde(default)]`
//! **without** one. `taxonomy_tests` pins every variant's wire string, its
//! derived class and its remediation key against the JSON these RPCs have
//! always emitted; a rename, a casing change or a re-ordered `class()` arm
//! fails there rather than in a user's status panel.
//!
//! # What deliberately did not come with them
//!
//! `classify_embed_error{,_str}` — the parser that turns an embed-stage error
//! *string* into a [`PipelineFailure`]. It rode the same `pub use` and has no
//! host caller: it reads the wording of `OpenAiEmbedding::embed`,
//! `OpenHumanCloudEmbedding::embed` and `tinyinference::embeddings::ollama`,
//! all of which run inside whichever engine ran the embed stage. Classifying
//! there is the point — the driver hands this host a code, not a message — so
//! the classifier stays with the stage and a copy here would be a second table
//! free to disagree with the one that actually labels the jobs.

use serde::{Deserialize, Serialize};

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
    /// Stable wire string — the same one the driver sends in
    /// [`DiagnosisFailure::class`](crate::openhuman::memory::api::provider::diagnosis::DiagnosisFailure)
    /// and the one `mem_tree_jobs.failure_class` persists.
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
    /// of retrying.
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
    ///
    /// Kept as an explicit table rather than derived from the serde attribute,
    /// because the two are read in different places — the string form is what
    /// a log line and `mem_tree_jobs.failure_reason` carry, the serde form is
    /// what the RPC body carries — and `taxonomy_tests` asserts they agree.
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

    /// Parses the stable wire string produced by [`Self::as_str`].
    ///
    /// Deliberately an inherent method returning `Option`, not a
    /// [`std::str::FromStr`] impl — the trait must return `Result`, and both
    /// callers ([`report::pipeline_failure`](super::report) and
    /// `rpc::blocking_cause`) treat "a code this build has never heard of" as
    /// *no cause to render*, not as an error to surface. An `Option` says that;
    /// a `Result` would invite someone to propagate it.
    #[allow(clippy::should_implement_trait)]
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
    ///
    /// [`LocalModelUnavailable`](Self::LocalModelUnavailable) is deliberately
    /// **transient** even though the user has to act: the condition (Ollama
    /// daemon stopped, model not pulled) clears from outside the app, and only
    /// transient rows are picked up by the driver's `requeue_transient_failed`
    /// — the automatic self-healing requeue. Classifying it unrecoverable would
    /// park every affected job until someone clicks "Retry failed" by hand, so
    /// a user who simply restarts Ollama would never see ingestion resume.
    pub fn class(self) -> FailureClass {
        match self {
            Self::Transient | Self::ExtractionTimeout | Self::LocalModelUnavailable => {
                FailureClass::Transient
            }
            _ => FailureClass::Unrecoverable,
        }
    }

    /// i18n key for the user-facing remediation. Embeddings causes lead
    /// with the local-Ollama path (the steered primary fix per spec FR-015).
    ///
    /// These strings are a **frontend contract** — the locale files key on them
    /// — so they are as much wire as the codes themselves.
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
/// remediation key (carried on the wire so the frontend stays presentational)
/// and an optional human-readable detail for logs/diagnosis.
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

    /// Attach a non-localized detail string (bounded by [`truncate_detail`];
    /// never log secrets).
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        self.detail = Some(truncate_detail(&detail));
        self
    }

    /// True when this failure should fail fast (no retry budget).
    pub fn is_unrecoverable(&self) -> bool {
        self.class == FailureClass::Unrecoverable
    }
}

impl std::fmt::Display for PipelineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.code.as_str(), self.class.as_str())?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PipelineFailure {}

/// Cap a detail string so a full provider response body can never balloon a
/// log line or a wire payload. Never contains a secret (it is an error body),
/// but keep it short anyway.
///
/// Counts **characters, not bytes**, so a multi-byte detail is never sliced
/// mid-codepoint. The cap is applied host-side as well as driver-side: the
/// driver already bounds `DiagnosisFailure::detail` with the same rule, so
/// [`report`](super::report) carries that value through rather than
/// re-truncating, and this only bites for details this host attaches itself
/// (the `unavailable` diagnostic's reason).
fn truncate_detail(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX).collect();
    format!("{truncated}…")
}

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
    /// the wire format backward-compatible (older clients omit it → `false`)
    /// and there is deliberately **no** `skip_serializing_if`, so the field is
    /// always emitted — which is what this RPC has always sent.
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

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod tests;
