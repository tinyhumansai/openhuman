//! Async host policy around the document module's `.docx` writer.
//!
//! The host keeps the typed contract and validation; OOXML synthesis runs in
//! the loadable document module. This wrapper owns the caller's deadline and
//! maps bus failures onto the agent-facing [`DocumentError`].
//!
//! This module supplies exactly that missing policy, and nothing else:
//!
//! 1. a `tokio::time::timeout` so a pathological input that slipped past
//!    validation cannot wedge the loop indefinitely, and
//! 2. the mapping from a bus error or elapsed deadline
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
#[path = "engine_tests.rs"]
mod tests;
