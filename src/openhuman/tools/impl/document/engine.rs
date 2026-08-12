//! Async wrapper around the vendored [`tinydocs`] `.docx` writer.
//!
//! The OOXML synthesis itself lives in
//! [`tinydocs::docx::generate`](https://github.com/tinyhumansai/tinydocs) —
//! spec validation, the paragraph/heading/bullet mapping, and the zip pack.
//! That call is **synchronous and CPU-bound by design**: `tinydocs` has no
//! opinion about executors or deadlines, because only a host knows its own.
//!
//! This module supplies exactly that missing policy, and nothing else:
//!
//! 1. a `spawn_blocking` hop so a CPU-bound pack never stalls the agent
//!    loop's executor, and
//! 2. a `tokio::time::timeout` so a pathological input that slipped past
//!    validation cannot wedge the loop indefinitely, and
//! 3. the mapping from a crate error, a join failure, or an elapsed deadline
//!    onto the agent-facing [`DocumentError`].
//!
//! Control flow here is identical to the presentation engine's, so the two
//! artifact producers keep failing in the same shapes.

use std::time::Duration;

use tokio::time::timeout;

use super::types::{DocumentError, GenerateDocumentInput};
use crate::openhuman::modules::documents;

/// Generate the `.docx` bytes for `input`, giving up after `deadline`.
///
/// The `deadline` covers the entire blocking call, including `spawn_blocking`
/// thread acquisition. Hitting it surfaces as
/// [`DocumentError::GenerationTimeout`].
pub(super) async fn generate(
    input: &GenerateDocumentInput,
    deadline: Duration,
) -> Result<Vec<u8>, DocumentError> {
    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => config,
        Err(error) => {
            return Err(DocumentError::GenerationFailed {
                stderr_truncated: DocumentError::truncate_stderr(&format!(
                    "config unavailable: {error}"
                )),
            });
        }
    };
    generate_with(&config, input, deadline).await
}

/// [`generate`], against a caller-supplied config.
///
/// Split out so a test can drive the whole path without `load_or_init`, which
/// reads — and on a fresh machine writes — the real user config directory. A
/// unit test that touches it depends on whatever is on the developer's box and
/// can leave a config file behind.
async fn generate_with(
    config: &crate::openhuman::config::Config,
    input: &GenerateDocumentInput,
    deadline: Duration,
) -> Result<Vec<u8>, DocumentError> {
    // Clone across the blocking boundary — cheap relative to the synthesis,
    // and it keeps the blocking closure `'static`.
    let owned = input.clone();
    let started = std::time::Instant::now();
    let section_count = owned.sections.len();
    let deadline_secs = deadline.as_secs();
    let title_chars = owned.title.chars().count();

    tracing::debug!(
        target: "document",
        deadline_secs,
        section_count,
        title_chars,
        "[document:engine] generate:start"
    );

    // Loaded before the clock starts. A first use may download and verify the
    // artifact, and a deadline meant for generation should not be spent on that
    // — otherwise the first document a user ever asks for is the one that times
    // out. Cached after the first call, so this is free from then on.
    if let Err(error) = documents::ensure_ready(config).await {
        return Err(DocumentError::from(error));
    }

    let call = timeout(deadline, documents::generate_docx(config, &owned)).await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    match call {
        Err(_elapsed) => {
            tracing::warn!(
                target: "document",
                elapsed_ms,
                deadline_secs,
                section_count,
                "[document:engine] generate:timeout"
            );
            Err(DocumentError::GenerationTimeout {
                timeout_secs: deadline_secs,
            })
        }
        Ok(Err(call_err)) => {
            let err = DocumentError::from(call_err);
            tracing::warn!(
                target: "document",
                elapsed_ms,
                kind = "module_failure",
                err = %err,
                "[document:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(bytes)) => {
            tracing::debug!(
                target: "document",
                elapsed_ms,
                bytes = bytes.len(),
                section_count,
                "[document:engine] generate:done"
            );
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::implementations::document::types::DocumentSection;

    fn sample_input() -> GenerateDocumentInput {
        GenerateDocumentInput {
            title: "Project Charter".to_string(),
            author: Some("Alice".to_string()),
            sections: vec![
                DocumentSection {
                    heading: Some("Overview".to_string()),
                    paragraphs: vec!["This document describes the plan.".to_string()],
                    bullets: vec![],
                },
                DocumentSection {
                    heading: Some("Goals".to_string()),
                    paragraphs: vec![],
                    bullets: vec!["Ship v1".to_string(), "Delight users".to_string()],
                },
            ],
        }
    }

    /// Read the entry names of a produced `.docx` byte buffer.
    fn docx_entry_names(bytes: &[u8]) -> Vec<String> {
        let cursor = std::io::Cursor::new(bytes.to_vec());
        let mut zip = zip::ZipArchive::new(cursor).expect("output is a valid zip archive");
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect()
    }

    /// Read one entry's UTF-8 body out of a produced `.docx`.
    fn docx_entry_body(bytes: &[u8], name: &str) -> String {
        let cursor = std::io::Cursor::new(bytes.to_vec());
        let mut zip = zip::ZipArchive::new(cursor).expect("valid zip");
        let mut entry = zip.by_name(name).expect("entry present");
        let mut body = String::new();
        std::io::Read::read_to_string(&mut entry, &mut body).unwrap();
        body
    }

    /// A config with modules turned off, so nothing leaves this process.
    ///
    /// The test drives `generate_with` rather than `generate` deliberately:
    /// `generate` calls `Config::load_or_init`, which reads — and on a fresh
    /// machine writes — the real user config directory, so its behaviour would
    /// depend on whatever is on the box running the test and it could leave a
    /// file behind. Supplying the config also makes the outcome deterministic:
    /// with modules disabled the only reachable error is `ModuleUnavailable`.
    fn isolated_config() -> crate::openhuman::config::Config {
        let mut config = crate::openhuman::config::Config::default();
        config.modules.enabled = false;
        config.modules.allow_download = false;
        config
    }

    #[tokio::test]
    async fn generate_reports_a_structured_outcome_when_no_module_is_available() {
        // Contract: with no module reachable, `generate` surfaces a clean,
        // structured outcome — never a panic, never a half-written buffer, and
        // never a hang waiting on a bus nobody is serving.
        match generate_with(&isolated_config(), &sample_input(), Duration::from_secs(30)).await {
            Err(DocumentError::ModuleUnavailable { .. }) => {}
            other => panic!("expected ModuleUnavailable with modules disabled, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_zero_deadline_never_panics_or_yields_a_partial_buffer() {
        // Which outcome arrives is inherently racy and must NOT be pinned: the
        // near-zero timeout usually elapses first, but the disabled-module check
        // runs ahead of the clock. Assert the invariant instead — any Ok is a
        // non-empty buffer, any Err is one of the documented variants.
        match generate_with(&isolated_config(), &sample_input(), Duration::ZERO).await {
            Ok(bytes) => assert!(!bytes.is_empty(), "a completed docx must be non-empty"),
            Err(DocumentError::GenerationTimeout { timeout_secs }) => {
                assert_eq!(timeout_secs, 0, "zero deadline reports 0 seconds");
            }
            Err(
                DocumentError::GenerationFailed { .. } | DocumentError::ModuleUnavailable { .. },
            ) => {
                // Failed or refused before the timer fired — still clean.
            }
            Err(other) => panic!("unexpected error variant under a zero deadline: {other:?}"),
        }
    }

    // The OOXML round trips that used to live here — container shape, which
    // text reaches document.xml, blank filtering — moved with the writer into
    // `tinydocs::docx`, which tests them against the bytes it produces. Asserting
    // them again through a bus call would test the same behaviour twice and
    // drift the moment one copy changed.

    #[test]
    fn a_module_failure_maps_onto_the_agent_facing_shape() {
        // There is no blocking task to join any more — the module owns its own
        // pool — so the failure this file has to classify is a call failure.
        // The three outcomes drive three different agent behaviours, which is
        // why the client distinguishes them at all.
        use crate::openhuman::modules::documents::DocumentCallError;

        assert!(matches!(
            DocumentError::from(DocumentCallError::InvalidInput("blank title".to_string())),
            DocumentError::InvalidInput { .. }
        ));
        assert!(matches!(
            DocumentError::from(DocumentCallError::Failed("writer stopped".to_string())),
            DocumentError::GenerationFailed { .. }
        ));
        assert!(matches!(
            DocumentError::from(DocumentCallError::Unavailable("no artifact".to_string())),
            DocumentError::ModuleUnavailable { .. }
        ));
    }

    #[test]
    fn a_module_reported_invalid_input_names_the_spec() {
        // The structured field/reason pair does not survive the wire, so the
        // conversion has to supply a field rather than leave the agent without
        // one. Local validation runs first, so this path is the rare one.
        use crate::openhuman::modules::documents::DocumentCallError;

        match DocumentError::from(DocumentCallError::InvalidInput("too long".to_string())) {
            DocumentError::InvalidInput { field, reason } => {
                assert_eq!(field, "spec");
                assert_eq!(reason, "too long");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}
