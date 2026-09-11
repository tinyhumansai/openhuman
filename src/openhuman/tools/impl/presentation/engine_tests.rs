//! What is left to test on this side of the bus.
//!
//! The deck shape, the image layout and the OOXML container are tested in
//! `crate::openhuman::tools::implementations::document::format::pptx`, where the code now lives — reproducing them here would
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
        format: crate::openhuman::tools::implementations::document::format::spec::ImageFormat::Png,
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
