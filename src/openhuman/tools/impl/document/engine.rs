//! Native Rust `.docx` generator, backed by the
//! [`docx-rs`](https://crates.io/crates/docx-rs) crate (MIT). Pure CPU,
//! no subprocess, no managed runtime — the document analogue of the
//! `ppt-rs`-backed presentation [`engine`](super::super::presentation).
//! Output is a byte buffer the caller writes to the artifact's
//! `output_path`.
//!
//! ## Mapping `GenerateDocumentInput` → OOXML
//!
//! The document is emitted as a linear paragraph stream:
//!
//! ```text
//! title             → bold, large "Title"-styled paragraph
//! author (opt)      → italic paragraph beneath the title
//! per section:
//!   heading (opt)   → bold "Heading1"-styled paragraph
//!   paragraphs[]    → one normal paragraph each
//!   bullets[]       → single-level `•` list (shared numbering id)
//! ```
//!
//! Headings carry BOTH a style id (`Title` / `Heading1`, which Word maps
//! to its built-in outline styles) AND an explicit bold+size run, so the
//! visual hierarchy survives even if a reader ignores the style table.
//! Empty / whitespace-only paragraphs and bullets are filtered so a
//! trailing blank entry does not emit an empty run.
//!
//! ## Runtime
//!
//! `docx-rs` synthesis is synchronous and CPU-bound (well under 100 ms
//! for the section cap). We still drive it through `spawn_blocking` so
//! the async executor is not blocked, and wrap the call in a
//! `tokio::time::timeout` so a runaway generation cannot wedge the agent
//! loop — identical control-flow to the presentation engine.

use std::time::Duration;

use docx_rs::{
    AbstractNumbering, Docx, IndentLevel, Level, LevelJc, LevelText, NumberFormat, Numbering,
    NumberingId, Paragraph, Run, Start,
};
use tokio::task::JoinError;
use tokio::time::{error::Elapsed, timeout};

use super::types::{DocumentError, GenerateDocumentInput};

/// Shared numbering id for the single-level bullet list. One abstract
/// numbering + one concrete numbering is registered on the document and
/// every bullet paragraph references it at indent level 0.
const BULLET_NUMBERING_ID: usize = 1;

/// Run font size for the document title, in OOXML half-points (28 pt).
const TITLE_SIZE_HALF_PT: usize = 56;
/// Run font size for a section heading, in half-points (16 pt).
const HEADING_SIZE_HALF_PT: usize = 32;
/// Run font size for the author byline, in half-points (12 pt).
const AUTHOR_SIZE_HALF_PT: usize = 24;

/// Run the synthesis. Returns the serialised `.docx` bytes ready to be
/// written to the artifact path.
///
/// The `deadline` covers the entire blocking call (including the
/// `spawn_blocking` thread acquisition). Hitting it surfaces as
/// [`DocumentError::GenerationTimeout`].
pub(super) async fn generate(
    input: &GenerateDocumentInput,
    deadline: Duration,
) -> Result<Vec<u8>, DocumentError> {
    // Clone the input across the blocking boundary — cheap relative to the
    // synthesis, and keeps the blocking closure `'static`.
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

    let join: Result<Result<Result<Vec<u8>, EngineFailure>, _>, Elapsed> = timeout(
        deadline,
        tokio::task::spawn_blocking(move || generate_blocking(&owned)),
    )
    .await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    match join {
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
        Ok(Err(join_err)) => {
            let err = map_join_error(join_err);
            tracing::warn!(
                target: "document",
                elapsed_ms,
                kind = "join_error",
                err = %err,
                "[document:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(Err(engine_err))) => {
            let err = map_engine_failure(engine_err);
            tracing::warn!(
                target: "document",
                elapsed_ms,
                kind = "engine_failure",
                err = %err,
                "[document:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(Ok(bytes))) => {
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

/// Blocking inner — runs on the `spawn_blocking` pool. Builds the whole
/// `docx-rs` document from the input then serialises it into an in-memory
/// zip buffer. Returns a dedicated [`EngineFailure`] so the async wrapper
/// can distinguish a library error from a panic / cancellation.
fn generate_blocking(input: &GenerateDocumentInput) -> Result<Vec<u8>, EngineFailure> {
    let docx = build_document(input);

    // `XMLDocx::pack` takes a `Write + Seek` writer by value; a
    // `&mut Cursor<Vec<u8>>` satisfies both traits, so we can pack into an
    // in-memory buffer and recover the bytes via `into_inner`.
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    docx.build()
        .pack(&mut cursor)
        .map_err(|err| EngineFailure::Library(format!("{err}")))?;
    Ok(cursor.into_inner())
}

/// Pure transformation from our schema to a `docx-rs` [`Docx`]. Pulled
/// out for unit-testability — the paragraph ordering + empty-filtering
/// rules are load-bearing for the rendered document shape.
fn build_document(input: &GenerateDocumentInput) -> Docx {
    // Register the shared single-level bullet list once. `NumberFormat`
    // "bullet" + a `•` level text renders an unordered list; every bullet
    // paragraph binds to this numbering id at indent level 0.
    let mut docx = Docx::new()
        .add_abstract_numbering(
            AbstractNumbering::new(BULLET_NUMBERING_ID).add_level(Level::new(
                0,
                Start::new(1),
                NumberFormat::new("bullet"),
                LevelText::new("•"),
                LevelJc::new("left"),
            )),
        )
        .add_numbering(Numbering::new(BULLET_NUMBERING_ID, BULLET_NUMBERING_ID));

    // Title — bold + large, styled as the built-in "Title" outline style.
    docx = docx.add_paragraph(
        Paragraph::new().style("Title").add_run(
            Run::new()
                .add_text(input.title.trim())
                .bold()
                .size(TITLE_SIZE_HALF_PT),
        ),
    );

    // Author byline — italic, if present and non-blank.
    if let Some(author) = input.author.as_deref().filter(|a| !a.trim().is_empty()) {
        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(author.trim())
                    .italic()
                    .size(AUTHOR_SIZE_HALF_PT),
            ),
        );
    }

    for section in &input.sections {
        if let Some(heading) = section.heading.as_deref().filter(|h| !h.trim().is_empty()) {
            docx = docx.add_paragraph(
                Paragraph::new().style("Heading1").add_run(
                    Run::new()
                        .add_text(heading.trim())
                        .bold()
                        .size(HEADING_SIZE_HALF_PT),
                ),
            );
        }
        for paragraph in &section.paragraphs {
            let text = paragraph.trim();
            if !text.is_empty() {
                docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(text)));
            }
        }
        for bullet in &section.bullets {
            let text = bullet.trim();
            if !text.is_empty() {
                docx = docx.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(text))
                        .numbering(NumberingId::new(BULLET_NUMBERING_ID), IndentLevel::new(0)),
                );
            }
        }
    }

    docx
}

/// Internal failure shape used to keep the blocking-thread surface
/// `Send`-clean (the underlying library error types are not guaranteed
/// to be `Send + Sync + 'static`).
#[derive(Debug)]
enum EngineFailure {
    Library(String),
}

fn map_engine_failure(failure: EngineFailure) -> DocumentError {
    match failure {
        EngineFailure::Library(msg) => DocumentError::GenerationFailed {
            stderr_truncated: DocumentError::truncate_stderr(&msg),
        },
    }
}

fn map_join_error(err: JoinError) -> DocumentError {
    // The outer `tokio::time::timeout` already routes the timeout case, so
    // a `JoinError` here is a panic (docx-rs bug / OOM on the blocking
    // pool) or a cancellation (runtime shutdown / explicit abort). Both
    // surface as `GenerationFailed` with context preserved — mirrors the
    // presentation engine so a "0s timeout" message is never fabricated.
    if err.is_panic() {
        DocumentError::GenerationFailed {
            stderr_truncated: DocumentError::truncate_stderr("document engine panicked"),
        }
    } else {
        DocumentError::GenerationFailed {
            stderr_truncated: DocumentError::truncate_stderr(&format!(
                "document engine task cancelled: {err}"
            )),
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

    #[tokio::test]
    async fn generate_round_trips_to_valid_docx() {
        // End-to-end: build → docx-rs → byte buffer → re-open as zip →
        // confirm the OOXML skeleton + that our text reached document.xml.
        let bytes = generate(&sample_input(), Duration::from_secs(30))
            .await
            .expect("generate should succeed");

        // A `.docx` is a zip: the magic bytes are the local-file-header
        // signature `PK\x03\x04`. This is the acceptance-criteria check
        // that any OOXML reader can open the file.
        assert!(
            bytes.len() > 200,
            "docx unexpectedly small ({} bytes)",
            bytes.len()
        );
        assert_eq!(&bytes[0..2], b"PK", "docx must start with the zip magic PK");

        let names = docx_entry_names(&bytes);
        for required in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
            assert!(
                names.iter().any(|n| n == required),
                "missing OOXML entry: {required} (got: {names:?})"
            );
        }

        // Numbering was used → the numbering part must materialise.
        assert!(
            names.iter().any(|n| n == "word/numbering.xml"),
            "bullet list should emit word/numbering.xml (got: {names:?})"
        );

        // Our title, heading, paragraph, and bullet text all reach the
        // rendered document body — none dropped on the floor.
        let doc = docx_entry_body(&bytes, "word/document.xml");
        for needle in [
            "Project Charter",
            "Overview",
            "This document describes the plan.",
            "Goals",
            "Ship v1",
            "Delight users",
        ] {
            assert!(
                doc.contains(needle),
                "document.xml missing text: {needle:?}"
            );
        }
    }

    #[tokio::test]
    async fn generate_drops_blank_paragraphs_and_bullets() {
        // Whitespace-only entries must not blow up generation and must not
        // emit empty runs — the engine trims + drops them.
        let input = GenerateDocumentInput {
            title: "Trimmed".to_string(),
            author: Some("   ".to_string()),
            sections: vec![DocumentSection {
                heading: Some("Kept".to_string()),
                paragraphs: vec!["real".to_string(), "   ".to_string(), String::new()],
                bullets: vec!["item".to_string(), "\t\n".to_string()],
            }],
        };
        let bytes = generate(&input, Duration::from_secs(30))
            .await
            .expect("generate should succeed on whitespace-only entries");
        let doc = docx_entry_body(&bytes, "word/document.xml");
        assert!(doc.contains("real"));
        assert!(doc.contains("item"));
    }

    #[tokio::test]
    async fn generate_yields_clean_structured_result_under_zero_deadline() {
        // Contract under an impossibly-short deadline: `generate` must surface a
        // clean, structured outcome — never a panic or a half-written buffer.
        //
        // Which outcome we get is inherently racy and must NOT be pinned: a
        // near-zero `timeout` wrapping `spawn_blocking` usually elapses first
        // (GenerationTimeout), but the runtime can instead cancel the blocking
        // task, which `map_join_error` maps to GenerationFailed, and a trivial
        // input can even finish before the timer fires (Ok). Asserting one exact
        // variant made this flake under coverage instrumentation. We assert the
        // real invariant: any Ok is a non-empty buffer, any Err is one of the
        // two documented structured variants, and nothing panics.
        match generate(&sample_input(), Duration::ZERO).await {
            Ok(bytes) => assert!(!bytes.is_empty(), "a completed docx must be non-empty"),
            Err(DocumentError::GenerationTimeout { timeout_secs }) => {
                assert_eq!(timeout_secs, 0, "zero deadline reports 0 seconds");
            }
            Err(DocumentError::GenerationFailed { .. }) => {
                // Blocking task cancelled before the timer fired — still clean.
            }
            Err(other) => panic!("unexpected error variant under a zero deadline: {other:?}"),
        }
    }

    #[tokio::test]
    async fn map_join_error_cancellation_becomes_generation_failed() {
        // A non-panic JoinError (cancellation via abort) surfaces as
        // GenerationFailed with the cancellation context preserved — never
        // a fabricated "0s timeout".
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        handle.abort();
        let join_err = handle.await.expect_err("aborted task yields JoinError");
        assert!(
            !join_err.is_panic(),
            "abort() yields a cancellation, not a panic"
        );
        match map_join_error(join_err) {
            DocumentError::GenerationFailed { stderr_truncated } => {
                assert!(
                    stderr_truncated.contains("document engine task cancelled"),
                    "cancellation context missing: {stderr_truncated:?}"
                );
            }
            other => panic!("expected GenerationFailed, got {other:?}"),
        }
    }
}
