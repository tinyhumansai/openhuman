//! Typed input / output / error contracts for the `generate_document` tool.
//!
//! The *document* half of these contracts — the section spec, its size
//! limits, and the validation rules — lives in the host-local
//! [`format`](super::format)
//! module and is re-exported here. Nothing about "a title, some sections, and a bullet
//! list" is OpenHuman-specific, so the definitions live where any host can
//! reach them and this module keeps only what genuinely is ours:
//!
//! - [`GenerateDocumentOutput`] — artifact ids and workspace paths, concepts
//!   the bus contract has no notion of.
//! - [`DocumentError`] — the agent-facing error shape, which carries a
//!   [`DocumentError::GenerationTimeout`] variant the document module cannot produce:
//!   the deadline is OpenHuman's policy, applied by [`engine`](super::engine)
//!   around a synchronous crate call.
//!
//! The re-exported [`GenerateDocumentInput`] is the format module's `DocumentSpec`
//! under its historical OpenHuman name. Field names are unchanged, so the JSON
//! tool schema the agent sees is byte-identical to before the extraction.

use serde::{Deserialize, Serialize};

use crate::openhuman::modules::documents::DocumentCallError;

// Reached through `spec`, not `docx`: this build carries the contract and not
// the writer, so the gated `docx` module is not compiled here at all. The types
// are the same ones the module validates against, which is the whole reason
// `spec` is separable.
pub use crate::openhuman::tools::implementations::document::format::spec::document::{
    DocumentSpec as GenerateDocumentInput, MAX_BULLETS_PER_SECTION, MAX_PARAGRAPHS_PER_SECTION,
    MAX_PARAGRAPH_CHARS, MAX_SECTIONS, MAX_TEXT_CHARS,
};

// `sections` is a `Vec<DocumentSection>`, so anything constructing an input
// needs the element type under this module's path. Today that is only test
// code (the tool itself deserialises whole inputs from JSON), and `mod types`
// is private, so the re-export reads as unused to the compiler — hence the
// explicit allow rather than dropping a name callers legitimately need.
#[allow(unused_imports)]
pub use crate::openhuman::tools::implementations::document::format::spec::DocumentSection;

/// Tool output returned via [`crate::openhuman::tools::traits::ToolResult`]
/// as the JSON `data` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateDocumentOutput {
    /// UUID of the persisted artifact record. Use with the
    /// `ai_get_artifact` / `ai_delete_artifact` RPCs.
    pub artifact_id: String,
    /// Absolute filesystem path to the generated `.docx`.
    pub artifact_path: String,
    /// Number of sections rendered into the document body.
    pub section_count: usize,
    /// On-disk size of the produced `.docx` in bytes.
    pub size_bytes: u64,
}

/// Structured error variants surfaced to the agent. Mirrors
/// [`presentation::types::PresentationError`](super::super::presentation)
/// so downstream error handling stays uniform across artifact producers.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("invalid input for field '{field}': {reason}")]
    InvalidInput { field: String, reason: String },

    #[error("document generation failed: {stderr_truncated}")]
    GenerationFailed { stderr_truncated: String },

    #[error("document generation exceeded {timeout_secs}s timeout")]
    GenerationTimeout { timeout_secs: u64 },

    /// The document module could not be loaded on this host.
    ///
    /// Terminal for the running process: the loader never retries a module that
    /// failed, so the message says what would change the outcome.
    #[error("document generation is unavailable: {reason}")]
    ModuleUnavailable { reason: String },
}

impl DocumentError {
    /// Truncate a library-error string to a 500-char cap (UTF-8-safe) so
    /// the variant never carries an unbounded payload back to the agent.
    /// Same cap/suffix as the presentation tool's `truncate_stderr`.
    pub(super) fn truncate_stderr(raw: &str) -> String {
        crate::openhuman::tools::implementations::document::format::Error::truncate_detail(raw)
    }
}

impl From<crate::openhuman::tools::implementations::document::format::Error> for DocumentError {
    /// Map a spec-validation failure onto the agent-facing shape.
    ///
    /// This is the *local* path: `validate_input` below checks the spec against
    /// the same limits the module would, before a call is made, so the agent
    /// gets a structured `field` / `reason` pair it can self-correct on without
    /// paying for a round trip. Errors that come back from the module take the
    /// `DocumentCallError` conversion instead, where that structure has already
    /// been flattened by the wire.
    ///
    /// `GenerationTimeout` is deliberately absent: validation has no deadline,
    /// so only [`engine`](super::engine) produces that variant.
    ///
    /// `crate::openhuman::tools::implementations::document::format::Error` is `#[non_exhaustive]`, so the catch-all arm is
    /// required by the compiler rather than chosen. It degrades a variant added
    /// by a future release to `GenerationFailed` carrying that variant's own
    /// `Display` text, and logs, so a crate bump that introduces a case worth
    /// handling structurally shows up rather than being swallowed.
    fn from(err: crate::openhuman::tools::implementations::document::format::Error) -> Self {
        match err {
            crate::openhuman::tools::implementations::document::format::Error::InvalidInput { field, reason } => Self::InvalidInput { field, reason },
            crate::openhuman::tools::implementations::document::format::Error::GenerationFailed { detail } => Self::GenerationFailed {
                stderr_truncated: detail,
            },
            other => {
                tracing::warn!(
                    target: "document",
                    err = %other,
                    "[document] unmapped tinydocs error variant; \
                     degrading to GenerationFailed"
                );
                Self::GenerationFailed {
                    stderr_truncated: Self::truncate_stderr(&other.to_string()),
                }
            }
        }
    }
}

/// Reported when the document module cannot be loaded on this host.
///
/// A distinct variant rather than a `GenerationFailed`, because the two mean
/// opposite things to whoever reads them: generation failing is a document that
/// might work on a retry, whereas an unavailable module means the capability is
/// not present in this build or on this machine and no amount of rephrasing the
/// request will produce one.
impl From<DocumentCallError> for DocumentError {
    /// Map a module-call failure onto the agent-facing shape.
    ///
    /// The three call outcomes map onto three different agent behaviours, which
    /// is the whole reason the client distinguishes them: `InvalidInput` is
    /// something a model can fix by rewriting its spec, `Failed` is not, and
    /// `Unavailable` means it should stop asking.
    ///
    /// The structured `field` / `reason` pair does not survive the bus — an
    /// error crosses as a name plus a message — so an `InvalidInput` from the
    /// module names `spec` and carries the message as its reason. In practice
    /// this is rare: [`validate_input`] runs against the same limits before the
    /// call is made, so a spec that reaches the module has already passed the
    /// checks the module would apply.
    fn from(err: DocumentCallError) -> Self {
        match err {
            DocumentCallError::InvalidInput(reason) => Self::InvalidInput {
                field: "spec".to_string(),
                reason: Self::truncate_stderr(&reason),
            },
            DocumentCallError::Unavailable(reason) => Self::ModuleUnavailable {
                reason: Self::truncate_stderr(&reason),
            },
            DocumentCallError::Failed(reason) => Self::GenerationFailed {
                stderr_truncated: Self::truncate_stderr(&reason),
            },
        }
    }
}

/// Validate the input early — before the blocking engine hop — so the agent
/// gets a structured `InvalidInput` it can self-correct on instead of a
/// generic engine error.
///
/// Delegates to `tinydocs`, which validates again inside
/// [`crate::openhuman::tools::implementations::document::format::docx::generate`]. The double check is intentional: validating
/// here lets the tool reject a bad call before allocating an artifact record,
/// and the crate-side check keeps `generate` safe for any other caller.
pub(super) fn validate_input(input: &GenerateDocumentInput) -> Result<(), DocumentError> {
    let title_chars = input.title.chars().count();
    let section_count = input.sections.len();
    tracing::debug!(
        target: "document",
        title_chars,
        section_count,
        "[document:types] validating input"
    );

    let result = input.validate().map_err(DocumentError::from);
    match &result {
        Ok(()) => tracing::debug!(
            target: "document",
            title_chars,
            section_count,
            "[document:types] input accepted"
        ),
        Err(DocumentError::InvalidInput { .. }) => tracing::debug!(
            target: "document",
            title_chars,
            section_count,
            error_kind = "invalid_input",
            "[document:types] input rejected"
        ),
        Err(DocumentError::GenerationFailed { .. }) => tracing::debug!(
            target: "document",
            title_chars,
            section_count,
            error_kind = "generation_failed",
            "[document:types] input rejected"
        ),
        // Neither of the last two can come out of validation, which has no
        // deadline and does not touch the module. Enumerated rather than caught
        // by a wildcard so a new variant is a compile error here — this match is
        // the one place every failure shape is named.
        Err(DocumentError::ModuleUnavailable { .. }) => tracing::debug!(
            target: "document",
            title_chars,
            section_count,
            error_kind = "module_unavailable",
            "[document:types] input rejected"
        ),
        Err(DocumentError::GenerationTimeout { .. }) => tracing::debug!(
            target: "document",
            title_chars,
            section_count,
            error_kind = "generation_timeout",
            "[document:types] input rejected"
        ),
    }
    result
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
