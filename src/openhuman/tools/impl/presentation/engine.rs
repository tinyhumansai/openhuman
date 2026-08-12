//! Async wrapper around the `tinydocs` module's `.pptx` writer.
//!
//! The synthesis itself — the slide mapping, the single-column image layout, the
//! EMU geometry — lives in `tinydocs::pptx` and runs inside the loaded module.
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

use tinydocs::spec::{WirePresentationSpec, WireSlideImage, WireSlideSpec};
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

    let config = match crate::openhuman::config::Config::load_or_init().await {
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

    // Loaded before the clock starts. A first use may download and verify the
    // artifact, and a deadline meant for generation should not be spent on that
    // — otherwise the first document a user ever asks for is the one that times
    // out. Cached after the first call, so this is free from then on.
    if let Err(error) = documents::ensure_ready(&config).await {
        return Err(PresentationError::from(error));
    }

    let call = timeout(deadline, documents::generate_pptx(&config, &deck, &payload)).await;

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
mod tests {
    //! What is left to test on this side of the bus.
    //!
    //! The deck shape, the image layout and the OOXML container are tested in
    //! `tinydocs::pptx`, where the code now lives — reproducing them here would
    //! assert the same behaviour twice and drift the moment one copy changed.
    //!
    //! What only exists here is [`build_request`]: the deck and the concatenated
    //! payload have to agree byte for byte, in order, or the module refuses the
    //! call. That agreement is this file's job.

    use super::*;
    use crate::openhuman::tools::implementations::presentation::types::SlideSpec;

    fn slide(title: &str) -> SlideSpec {
        SlideSpec {
            title: title.to_string(),
            body: Some("Body".to_string()),
            bullets: vec!["Bullet".to_string()],
            speaker_notes: Some("Notes".to_string()),
            images: vec![],
        }
    }

    fn input(slides: Vec<SlideSpec>) -> GeneratePresentationInput {
        GeneratePresentationInput {
            title: "Quarterly".to_string(),
            author: Some("Alice".to_string()),
            theme: Some("plain".to_string()),
            slides,
        }
    }

    fn resolved(bytes: &[u8], caption: Option<&str>) -> ResolvedSlideImage {
        ResolvedSlideImage {
            bytes: bytes.to_vec(),
            format: tinydocs::spec::ImageFormat::Png,
            width_px: 4,
            height_px: 4,
            caption: caption.map(str::to_string),
        }
    }

    #[test]
    fn the_wire_deck_carries_every_text_field() {
        let (deck, payload) = build_request(&input(vec![slide("First")]), &[]);
        assert_eq!(deck.title, "Quarterly");
        assert_eq!(deck.author.as_deref(), Some("Alice"));
        assert_eq!(deck.theme.as_deref(), Some("plain"));
        assert_eq!(deck.slides.len(), 1);
        assert_eq!(deck.slides[0].title, "First");
        assert_eq!(deck.slides[0].body.as_deref(), Some("Body"));
        assert_eq!(deck.slides[0].bullets, vec!["Bullet".to_string()]);
        assert_eq!(deck.slides[0].speaker_notes.as_deref(), Some("Notes"));
        assert!(payload.is_empty(), "a text-only deck sends no image bytes");
    }

    #[test]
    fn declared_lengths_slice_the_payload_back_into_the_original_images() {
        // The property the module relies on: walking the deck's byte_lens in
        // order must reproduce exactly the images that went in. If this drifts,
        // a deck renders with pictures assembled from two different images.
        let first = vec![1u8; 10];
        let second = vec![2u8; 25];
        let third = vec![3u8; 7];
        let images = vec![
            vec![resolved(&first, Some("one")), resolved(&second, None)],
            vec![resolved(&third, Some("three"))],
        ];
        let (deck, payload) = build_request(&input(vec![slide("A"), slide("B")]), &images);

        assert_eq!(payload.len(), first.len() + second.len() + third.len());
        let mut cursor = 0usize;
        let mut seen = Vec::new();
        for wire_slide in &deck.slides {
            for image in &wire_slide.images {
                let len = image.byte_len as usize;
                seen.push(payload[cursor..cursor + len].to_vec());
                cursor += len;
            }
        }
        assert_eq!(
            cursor,
            payload.len(),
            "the lengths must consume the payload"
        );
        assert_eq!(seen, vec![first, second, third]);
    }

    #[test]
    fn captions_survive_onto_the_wire_images() {
        let images = vec![vec![
            resolved(&[9u8; 3], Some("A chart")),
            resolved(&[8u8; 3], None),
        ]];
        let (deck, _) = build_request(&input(vec![slide("A")]), &images);
        assert_eq!(deck.slides[0].images[0].caption.as_deref(), Some("A chart"));
        assert_eq!(deck.slides[0].images[1].caption, None);
    }

    #[test]
    fn a_slide_with_no_resolved_images_declares_none() {
        // `resolve_images` skips an unreadable image with a warning rather than
        // failing the deck, so a slide can arrive here with fewer images than it
        // asked for — and the deck must declare what is actually being sent.
        let images = vec![vec![]];
        let (deck, payload) = build_request(&input(vec![slide("A")]), &images);
        assert!(deck.slides[0].images.is_empty());
        assert!(payload.is_empty());
    }

    #[test]
    fn a_short_images_argument_leaves_later_slides_imageless() {
        // Defensive: `images` is indexed by slide, and a caller that passes a
        // shorter vector must not panic or shift images onto the wrong slide.
        let images = vec![vec![resolved(&[5u8; 4], None)]];
        let (deck, payload) = build_request(&input(vec![slide("A"), slide("B")]), &images);
        assert_eq!(deck.slides[0].images.len(), 1);
        assert!(deck.slides[1].images.is_empty());
        assert_eq!(payload.len(), 4);
    }

    #[test]
    fn a_module_failure_maps_onto_the_agent_facing_shape() {
        use crate::openhuman::modules::documents::DocumentCallError;

        assert!(matches!(
            PresentationError::from(DocumentCallError::InvalidInput("bad".to_string())),
            PresentationError::InvalidInput { .. }
        ));
        assert!(matches!(
            PresentationError::from(DocumentCallError::Failed("writer stopped".to_string())),
            PresentationError::GenerationFailed { exit_code: -1, .. }
        ));
        assert!(matches!(
            PresentationError::from(DocumentCallError::Unavailable("no artifact".to_string())),
            PresentationError::ModuleUnavailable { .. }
        ));
    }
}
