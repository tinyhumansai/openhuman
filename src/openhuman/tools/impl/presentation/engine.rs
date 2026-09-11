//! Async wrapper around the `tinydocs` module's `.pptx` writer.
//!
//! The synthesis itself — the slide mapping, the single-column image layout, the
//! EMU geometry — lives in `crate::openhuman::tools::implementations::document::format::pptx` and runs inside the loaded module.
//! What is left here is the policy only a host can supply:
//!
//! 1. a deadline, because the module holds no opinion about how long a caller
//!    is willing to wait, and
//! 2. the mapping from a module-call failure or an elapsed deadline onto the
//!    agent-facing [`PresentationError`].
//!
//! There is no `spawn_blocking` hop any more: the module owns its own blocking
//! pool, so the CPU-bound pack never runs on this executor to begin with.
//!
//! # Images cross as one stream
//!
//! A deck's images are concatenated in slide order and sent on a single bus
//! stream, with each image's length declared in the wire spec. Images cannot
//! ride inside the call: a frame is a 16 MiB JSON document and a deck may
//! legally carry 40 MiB of pictures.
//!
//! Resolution stays on this side — reading an artifact, checking a path against
//! the security policy — because it is host policy the module must not hold.

use std::time::Duration;

use crate::openhuman::tools::implementations::document::format::spec::{
    WirePresentationSpec, WireSlideImage, WireSlideSpec,
};
use tokio::time::timeout;

use super::types::{GeneratePresentationInput, PresentationError, ResolvedSlideImage};
use crate::openhuman::modules::documents;

/// Run the synthesis. Returns the serialised `.pptx` bytes ready to be written
/// to the artifact path.
///
/// The `deadline` covers the whole call, including the image transfer. Hitting
/// it surfaces as [`PresentationError::GenerationTimeout`].
pub(super) async fn generate(
    input: &GeneratePresentationInput,
    images: &[Vec<ResolvedSlideImage>],
    deadline: Duration,
    config: Option<&crate::openhuman::config::Config>,
) -> Result<Vec<u8>, PresentationError> {
    let (deck, payload) = build_request(input, images);
    let started = std::time::Instant::now();
    let slide_count = deck.slides.len();
    let deadline_secs = deadline.as_secs();
    let image_bytes = payload.len();

    tracing::debug!(
        target: "presentation",
        deadline_secs,
        slide_count,
        image_bytes,
        title_chars = input.title.chars().count(),
        "[presentation:engine] generate:start"
    );

    let loaded_config;
    let config = match config {
        Some(config) => config,
        None => {
            loaded_config = match crate::openhuman::config::Config::load_or_init().await {
                Ok(config) => config,
                Err(error) => {
                    return Err(PresentationError::GenerationFailed {
                        exit_code: -1,
                        stderr_truncated: PresentationError::truncate_stderr(&format!(
                            "config unavailable: {error}"
                        )),
                    });
                }
            };
            &loaded_config
        }
    };

    // Loaded before the clock starts. A first use may download and verify the
    // artifact, and a deadline meant for generation should not be spent on that
    // — otherwise the first document a user ever asks for is the one that times
    // out. Cached after the first call, so this is free from then on.
    if let Err(error) = documents::ensure_ready(config).await {
        return Err(PresentationError::from(error));
    }

    let call = timeout(deadline, documents::generate_pptx(config, &deck, &payload)).await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    match call {
        Err(_elapsed) => {
            tracing::warn!(
                target: "presentation",
                elapsed_ms,
                deadline_secs,
                slide_count,
                "[presentation:engine] generate:timeout"
            );
            Err(PresentationError::GenerationTimeout {
                timeout_secs: deadline_secs,
            })
        }
        Ok(Err(call_err)) => {
            let err = PresentationError::from(call_err);
            tracing::warn!(
                target: "presentation",
                elapsed_ms,
                kind = "module_failure",
                err = %err,
                "[presentation:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(bytes)) => {
            tracing::debug!(
                target: "presentation",
                elapsed_ms,
                bytes = bytes.len(),
                slide_count,
                "[presentation:engine] generate:done"
            );
            Ok(bytes)
        }
    }
}

/// Turn the tool's input and its resolved images into the wire deck plus the
/// concatenated image payload.
///
/// The two have to agree: every `byte_len` in the deck is the length of the
/// corresponding slice in `payload`, in the same order, and the module refuses
/// the call if they do not add up. Building both here, in one pass, is what
/// keeps them consistent.
fn build_request(
    input: &GeneratePresentationInput,
    images: &[Vec<ResolvedSlideImage>],
) -> (WirePresentationSpec, Vec<u8>) {
    let mut payload = Vec::new();
    let mut slides = Vec::with_capacity(input.slides.len());

    for (index, slide) in input.slides.iter().enumerate() {
        let resolved = images.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let mut wire_images = Vec::with_capacity(resolved.len());
        for image in resolved {
            payload.extend_from_slice(&image.bytes);
            wire_images.push(WireSlideImage {
                byte_len: image.bytes.len() as u64,
                caption: image.caption.clone(),
            });
        }
        slides.push(WireSlideSpec {
            title: slide.title.clone(),
            body: slide.body.clone(),
            bullets: slide.bullets.clone(),
            speaker_notes: slide.speaker_notes.clone(),
            images: wire_images,
        });
    }

    (
        WirePresentationSpec {
            title: input.title.clone(),
            author: input.author.clone(),
            theme: input.theme.clone(),
            slides,
        },
        payload,
    )
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
