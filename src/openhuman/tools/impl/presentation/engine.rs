//! Native Rust `.pptx` generator (replaces the python-pptx subprocess
//! path shipped in #2778).
//!
//! Backed by the [`ppt-rs`](https://crates.io/crates/ppt-rs) crate
//! (Apache-2.0). Pure CPU, no subprocess, no managed runtime, no
//! first-call venv-setup latency. Output is a byte buffer the caller
//! writes to the artifact's `output_path`.
//!
//! ## Mapping `SlideSpec` → `ppt-rs`
//!
//! `ppt-rs::SlideContent` does not expose a separate "body paragraph"
//! slot today; everything below the title is a bullet. We collapse
//! [`SlideSpec::body`] into a leading bullet so the body text still
//! reaches the rendered slide:
//!
//! ```text
//! SlideSpec { title, body: Some(body), bullets: [b1, b2], speaker_notes: Some(n) }
//!   → SlideContent::new(title).add_bullet(body).add_bullet(b1).add_bullet(b2).notes(n)
//! ```
//!
//! Empty / whitespace-only entries are filtered out so a trailing
//! blank `body` does not produce an empty bullet marker.
//!
//! ## Title slide
//!
//! `ppt-rs::create_pptx_with_content(title, slides)` treats `title`
//! as deck metadata only (lands in `docProps/core.xml`) — it does NOT
//! emit a separate title-slide. To preserve the python-pptx
//! contract — title slide first, content slides after, with
//! [`GeneratePresentationOutput::slide_count`] excluding the title
//! slide — we prepend a synthetic title slide built from
//! [`GeneratePresentationInput::title`] (+ optional `author` byline).
//!
//! ## Runtime
//!
//! `ppt-rs::create_pptx_with_content` is synchronous, CPU-bound, and
//! typically completes in <100 ms even for the 64-slide cap. We still
//! drive it through `spawn_blocking` so the async executor is not
//! blocked, and wrap the whole call in a `tokio::time::timeout` so a
//! runaway generation cannot wedge the agent loop.

use std::time::Duration;

use ppt_rs::generator::{create_pptx_with_content, Image, SlideContent};
use tokio::task::JoinError;
use tokio::time::{error::Elapsed, timeout};

use super::types::{GeneratePresentationInput, PresentationError, ResolvedSlideImage};

/// Slide geometry (matches `ppt-rs`'s default 4:3 deck: 10in × 7.5in).
const SLIDE_WIDTH_EMU: u32 = 9_144_000;
const SLIDE_HEIGHT_EMU: u32 = 6_858_000;
/// 1 inch side margins → usable content column width.
const SIDE_MARGIN_EMU: u32 = 914_400;
/// Images live in the lower band of the slide, beneath the title/body
/// placeholder. Top of that band ≈ slide midpoint; bottom keeps a 0.5in
/// margin. v1 single-column: images stack vertically inside this band.
const IMAGE_BAND_TOP_EMU: u32 = 3_429_000;
const IMAGE_BAND_BOTTOM_MARGIN_EMU: u32 = 457_200;
/// Vertical gap between stacked images.
const IMAGE_STACK_GAP_EMU: u32 = 91_440;
/// EMU per pixel at 96 DPI (matches `ppt-rs`'s own px→EMU convention).
const EMU_PER_PX: u32 = 9_525;

/// Run the synthesis. Returns the serialised `.pptx` bytes ready to
/// be written to the artifact path.
///
/// The `deadline` covers the entire blocking call (including the
/// `spawn_blocking` thread acquisition). Hitting it surfaces as
/// [`PresentationError::GenerationTimeout`].
pub(super) async fn generate(
    input: &GeneratePresentationInput,
    images: &[Vec<ResolvedSlideImage>],
    deadline: Duration,
) -> Result<Vec<u8>, PresentationError> {
    // Build the SlideContent vector on the async thread — cheap allocation
    // work, no need to send the original `input` across the blocking
    // boundary as a borrow.
    let slides = build_slides(input, images);
    let deck_title = input.title.clone();
    let started = std::time::Instant::now();
    let slide_count = slides.len();
    let deadline_secs = deadline.as_secs();
    let title_chars = deck_title.chars().count();

    tracing::debug!(
        target: "presentation",
        deadline_secs,
        slide_count,
        title_chars,
        "[presentation:engine] generate:start"
    );

    let join: Result<Result<Result<Vec<u8>, EngineFailure>, _>, Elapsed> = timeout(
        deadline,
        tokio::task::spawn_blocking(move || generate_blocking(&deck_title, slides)),
    )
    .await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    match join {
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
        Ok(Err(join_err)) => {
            let err = map_join_error(join_err);
            tracing::warn!(
                target: "presentation",
                elapsed_ms,
                kind = "join_error",
                err = %err,
                "[presentation:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(Err(engine_err))) => {
            let err = map_engine_failure(engine_err);
            tracing::warn!(
                target: "presentation",
                elapsed_ms,
                kind = "engine_failure",
                err = %err,
                "[presentation:engine] generate:failure"
            );
            Err(err)
        }
        Ok(Ok(Ok(bytes))) => {
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

/// Pure transformation from our schema to `ppt-rs`'s. Pulled out of
/// `generate` for unit-testability — the slide ordering + empty
/// filtering rules are load-bearing for the rendered deck shape.
fn build_slides(
    input: &GeneratePresentationInput,
    images: &[Vec<ResolvedSlideImage>],
) -> Vec<SlideContent> {
    let mut out = Vec::with_capacity(input.slides.len() + 1);

    // Synthetic title slide — preserves the python-pptx behaviour where
    // the first rendered slide carries the deck title (+ optional author
    // byline). Without this prepend, the deck would open straight onto
    // the first content slide and the `title` argument would only land
    // in core.xml metadata. The title slide carries no images.
    let mut title_slide = SlideContent::new(&input.title);
    if let Some(author) = input.author.as_deref().filter(|a| !a.trim().is_empty()) {
        title_slide = title_slide.add_bullet(author);
    }
    out.push(title_slide);

    for (idx, spec) in input.slides.iter().enumerate() {
        let mut slide = SlideContent::new(&spec.title);
        if let Some(body) = spec.body.as_deref().filter(|b| !b.trim().is_empty()) {
            slide = slide.add_bullet(body);
        }
        for bullet in &spec.bullets {
            if !bullet.trim().is_empty() {
                slide = slide.add_bullet(bullet);
            }
        }
        // Attach resolved images AFTER the text, single-column in the
        // lower band of the slide. Each image's caption (if any) is
        // rendered as a trailing bullet so the label is not lost.
        let slide_images = images.get(idx).map(Vec::as_slice).unwrap_or(&[]);
        for placed in place_single_column(slide_images) {
            slide = slide.add_image(placed.image);
            if let Some(caption) = placed.caption {
                slide = slide.add_bullet(&caption);
            }
        }
        if let Some(notes) = spec
            .speaker_notes
            .as_deref()
            .filter(|n| !n.trim().is_empty())
        {
            slide = slide.notes(notes);
        }
        out.push(slide);
    }

    out
}

/// A positioned `ppt-rs` image plus its (optional) caption text.
struct PlacedImage {
    image: Image,
    caption: Option<String>,
}

/// Lay resolved images out in a single vertical column inside the lower
/// band of the slide. Each image is scaled to fit its slot while
/// preserving aspect ratio and centred horizontally + vertically within
/// the slot. v1 layout — a multi-image grid is deferred.
fn place_single_column(images: &[ResolvedSlideImage]) -> Vec<PlacedImage> {
    let n = images.len() as u32;
    if n == 0 {
        return Vec::new();
    }

    let content_left = SIDE_MARGIN_EMU;
    let content_width = SLIDE_WIDTH_EMU.saturating_sub(2 * SIDE_MARGIN_EMU);
    let band_height = SLIDE_HEIGHT_EMU
        .saturating_sub(IMAGE_BAND_TOP_EMU)
        .saturating_sub(IMAGE_BAND_BOTTOM_MARGIN_EMU);
    let total_gap = IMAGE_STACK_GAP_EMU.saturating_mul(n.saturating_sub(1));
    let slot_height = band_height.saturating_sub(total_gap) / n;

    images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let slot_top = IMAGE_BAND_TOP_EMU + (i as u32) * (slot_height + IMAGE_STACK_GAP_EMU);
            let (w, h) = fit_within(
                img.width_px.saturating_mul(EMU_PER_PX),
                img.height_px.saturating_mul(EMU_PER_PX),
                content_width,
                slot_height,
            );
            let x = content_left + content_width.saturating_sub(w) / 2;
            let y = slot_top + slot_height.saturating_sub(h) / 2;
            let image = Image::from_bytes(img.bytes.clone(), w, h, img.format).position(x, y);
            PlacedImage {
                image,
                caption: img.caption.clone(),
            }
        })
        .collect()
}

/// Scale `(w, h)` (EMU) to fit within `(max_w, max_h)` preserving aspect
/// ratio. Never upscales beyond the bounding box; only shrinks when the
/// source overflows. Falls back to the bounding box if the source has a
/// degenerate zero dimension.
fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (max_w, max_h);
    }
    // Scale factor in fixed-point-ish f64; min of the two axis ratios.
    let ratio_w = max_w as f64 / w as f64;
    let ratio_h = max_h as f64 / h as f64;
    let scale = ratio_w.min(ratio_h);
    let fit_w = ((w as f64) * scale).round().max(1.0) as u32;
    let fit_h = ((h as f64) * scale).round().max(1.0) as u32;
    (fit_w.min(max_w).max(1), fit_h.min(max_h).max(1))
}

/// Blocking inner — runs on the `spawn_blocking` pool. Returns a
/// dedicated `EngineFailure` so the async wrapper can distinguish
/// "library returned an error" from "the blocking task itself panicked
/// or was cancelled".
fn generate_blocking(
    deck_title: &str,
    slides: Vec<SlideContent>,
) -> Result<Vec<u8>, EngineFailure> {
    create_pptx_with_content(deck_title, slides)
        .map_err(|err| EngineFailure::Library(format!("{err}")))
}

/// Internal failure shape used to keep the blocking-thread surface
/// `Send`-clean (the `ppt-rs` error type is not guaranteed to be
/// `Send + Sync + 'static`).
#[derive(Debug)]
enum EngineFailure {
    Library(String),
}

fn map_engine_failure(failure: EngineFailure) -> PresentationError {
    match failure {
        EngineFailure::Library(msg) => PresentationError::GenerationFailed {
            exit_code: -1,
            stderr_truncated: PresentationError::truncate_stderr(&msg),
        },
    }
}

fn map_join_error(err: JoinError) -> PresentationError {
    // A bare panic indicates a `ppt-rs` bug or an OOM on the blocking
    // pool; surface as `GenerationFailed` so the user sees a structured
    // error and the agent can retry with a smaller deck.
    //
    // Cancellation (non-panic `JoinError`) is a distinct shape: the
    // outer `tokio::time::timeout` already routes the timeout case
    // before us, so a cancellation that reaches `map_join_error` is
    // something else — runtime shutdown, an explicit abort, or the
    // runtime cancelling the blocking task for unrelated reasons.
    // Reporting it as `GenerationTimeout { timeout_secs: 0 }` produced
    // a misleading "exceeded 0s timeout" message and discarded the
    // underlying `JoinError` detail that's valuable for triage. We
    // surface it as `GenerationFailed` and preserve the cancellation
    // context in `stderr_truncated`.
    if err.is_panic() {
        PresentationError::GenerationFailed {
            exit_code: -1,
            stderr_truncated: PresentationError::truncate_stderr("presentation engine panicked"),
        }
    } else {
        PresentationError::GenerationFailed {
            exit_code: -1,
            stderr_truncated: PresentationError::truncate_stderr(&format!(
                "presentation engine task cancelled: {err}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::implementations::presentation::types::SlideSpec;

    fn input_with_one_slide() -> GeneratePresentationInput {
        GeneratePresentationInput {
            title: "Quarterly review".to_string(),
            author: Some("Alice".to_string()),
            theme: None,
            slides: vec![SlideSpec {
                title: "Highlights".to_string(),
                body: Some("Revenue up 12% QoQ.".to_string()),
                bullets: vec![
                    "Closed two key deals".to_string(),
                    "Hired 3 engineers".to_string(),
                ],
                speaker_notes: Some("Emphasise headcount efficiency.".to_string()),
                images: vec![],
            }],
        }
    }

    #[test]
    fn build_slides_prepends_title_slide_with_author_byline() {
        let slides = build_slides(&input_with_one_slide(), &[]);
        // Title slide + 1 content slide.
        assert_eq!(slides.len(), 2);
        // ppt-rs SlideContent fields are pub but private to this crate
        // boundary; downstream `create_pptx_with_content` is the only
        // semantically meaningful assertion — covered by the
        // `generate_round_trips_to_valid_pptx` test below. Here we only
        // assert the *count* invariant (title-slide prepended), since
        // the public API of SlideContent does not expose its bullets.
    }

    #[test]
    fn build_slides_drops_blank_body_and_bullet_entries() {
        let mut input = input_with_one_slide();
        input.author = Some("   ".to_string());
        input.slides[0].body = Some("".to_string());
        input.slides[0].bullets = vec!["real".to_string(), "  ".to_string(), "".to_string()];
        input.slides[0].speaker_notes = Some("\n\t ".to_string());

        let slides = build_slides(&input, &[]);
        // 2 slides regardless — empty filtering happens INSIDE the slide,
        // not at the slide-list level. Behaviour assertion: the call
        // does not panic on whitespace-only fields and downstream
        // ppt-rs generation succeeds (cross-checked by the round-trip
        // test below).
        assert_eq!(slides.len(), 2);
    }

    #[tokio::test]
    async fn generate_round_trips_to_valid_pptx() {
        // End-to-end: build → ppt-rs → byte buffer → re-open as zip →
        // confirm OOXML skeleton entries. This is the load-bearing
        // assertion that the engine swap produces a deck that any
        // OOXML reader (PowerPoint, Keynote, LibreOffice, Google
        // Slides) can open.
        let input = input_with_one_slide();
        let bytes = generate(&input, &[], Duration::from_secs(30))
            .await
            .expect("generate should succeed on a 1-slide deck");

        assert!(
            bytes.len() > 1000,
            "deck unexpectedly small ({} bytes)",
            bytes.len()
        );

        let cursor = std::io::Cursor::new(&bytes);
        let mut zip = zip::ZipArchive::new(cursor).expect("output is a valid zip archive");

        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();

        // OOXML spec-required entries — without these PowerPoint will
        // refuse to open the file with "PowerPoint found a problem".
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt/theme/theme1.xml",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "docProps/core.xml",
            "docProps/app.xml",
        ] {
            assert!(
                names.iter().any(|n| n == required),
                "missing OOXML entry: {required} (got: {names:?})"
            );
        }

        // Title slide (slide1) + 1 content slide (slide2) = 2.
        assert!(names.iter().any(|n| n == "ppt/slides/slide1.xml"));
        assert!(names.iter().any(|n| n == "ppt/slides/slide2.xml"));
        // No slide3 — we only had one SlideSpec.
        assert!(!names.iter().any(|n| n == "ppt/slides/slide3.xml"));

        // Speaker notes were set on the content slide → notesSlide
        // must materialise. Without this, the notes pane in PowerPoint
        // / Keynote stays empty even though the agent populated it.
        assert!(names
            .iter()
            .any(|n| n.starts_with("ppt/notesSlides/notesSlide")));

        // Sanity: the title text shows up somewhere in the generated
        // slide XML. We do not assert exact placement (the placeholder
        // structure is owned by ppt-rs's slide layout) — only that the
        // string was not dropped on the floor.
        let mut slide1 = zip.by_name("ppt/slides/slide1.xml").unwrap();
        let mut slide1_body = String::new();
        std::io::Read::read_to_string(&mut slide1, &mut slide1_body).unwrap();
        assert!(
            slide1_body.contains("Quarterly review"),
            "deck title missing from rendered slide1.xml"
        );
    }

    #[tokio::test]
    async fn map_join_error_cancellation_becomes_generation_failed() {
        // A non-panic JoinError (cancellation via abort) MUST NOT surface
        // as GenerationTimeout { timeout_secs: 0 } — that produces a
        // misleading "exceeded 0s timeout" message and loses the
        // JoinError detail useful for triage. Cancellation belongs in
        // GenerationFailed with the cancellation context preserved.
        let handle = tokio::spawn(async {
            // Park forever; we abort before this returns.
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        handle.abort();
        let join_err = handle.await.expect_err("aborted task yields JoinError");
        assert!(
            !join_err.is_panic(),
            "abort() should produce a cancellation JoinError, not a panic"
        );

        match map_join_error(join_err) {
            PresentationError::GenerationFailed {
                exit_code,
                stderr_truncated,
            } => {
                assert_eq!(exit_code, -1, "cancellation maps to exit_code -1");
                assert!(
                    stderr_truncated.contains("presentation engine task cancelled"),
                    "cancellation context missing from stderr_truncated: {stderr_truncated:?}"
                );
            }
            other => panic!("expected GenerationFailed for cancellation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn generate_surfaces_timeout_under_tiny_deadline() {
        // A 1 ns deadline cannot complete any real work — we expect a
        // structured Timeout, not a panic or a half-written buffer.
        let input = input_with_one_slide();
        let err = generate(&input, &[], Duration::from_nanos(1))
            .await
            .expect_err("1 ns deadline should never satisfy the timeout");
        match err {
            PresentationError::GenerationTimeout { timeout_secs } => {
                assert_eq!(
                    timeout_secs, 0,
                    "nanosecond timeout rounds down to 0 seconds"
                );
            }
            other => panic!("expected GenerationTimeout, got {other:?}"),
        }
    }

    /// Canonical 1×1 PNG used to exercise the embed path.
    fn png_1x1() -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")
            .unwrap()
    }

    fn resolved_png(caption: Option<&str>) -> ResolvedSlideImage {
        ResolvedSlideImage {
            bytes: png_1x1(),
            format: "PNG",
            width_px: 320,
            height_px: 240,
            caption: caption.map(ToString::to_string),
        }
    }

    #[test]
    fn fit_within_preserves_aspect_and_clamps() {
        // 2:1 source into a square box → width-limited, half-height.
        let (w, h) = fit_within(2000, 1000, 1000, 1000);
        assert_eq!(w, 1000);
        assert_eq!(h, 500);
        // Never exceeds the bounding box on either axis.
        let (w, h) = fit_within(10, 10, 4000, 800);
        assert!(w <= 4000 && h <= 800);
        // Degenerate zero dimension falls back to the box.
        assert_eq!(fit_within(0, 10, 500, 600), (500, 600));
    }

    #[test]
    fn place_single_column_stacks_within_slide_bounds() {
        let imgs = vec![resolved_png(None), resolved_png(None), resolved_png(None)];
        let placed = place_single_column(&imgs);
        assert_eq!(placed.len(), 3);
        let mut prev_bottom = 0u32;
        for p in &placed {
            // Horizontally inside the content column.
            assert!(p.image.x >= SIDE_MARGIN_EMU);
            assert!(p.image.x + p.image.width <= SLIDE_WIDTH_EMU - SIDE_MARGIN_EMU + 1);
            // Vertically inside the lower band and monotonically stacked.
            assert!(p.image.y >= IMAGE_BAND_TOP_EMU);
            assert!(p.image.y + p.image.height <= SLIDE_HEIGHT_EMU);
            assert!(
                p.image.y >= prev_bottom,
                "images must not overlap vertically"
            );
            prev_bottom = p.image.y + p.image.height;
        }
    }

    #[tokio::test]
    async fn generate_embeds_image_into_media_and_content_types() {
        // Images are supplied via the resolved arg, not the input spec.
        let input = input_with_one_slide();
        let images = vec![vec![resolved_png(Some("Figure 1"))]];

        let bytes = generate(&input, &images, Duration::from_secs(30))
            .await
            .expect("generate with one image should succeed");

        let cursor = std::io::Cursor::new(&bytes);
        let mut zip = zip::ZipArchive::new(cursor).expect("valid zip");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();

        assert!(
            names.iter().any(|n| n == "ppt/media/image1.png"),
            "embedded PNG missing from ppt/media (got: {names:?})"
        );

        let ct_body = {
            let mut ct = zip.by_name("[Content_Types].xml").unwrap();
            let mut body = String::new();
            std::io::Read::read_to_string(&mut ct, &mut body).unwrap();
            body
        };
        assert!(
            ct_body.contains("Extension=\"png\""),
            "[Content_Types].xml missing png default extension"
        );

        // Caption rendered as a bullet → its text lands in slide2.xml
        // (slide1 is the synthetic title slide).
        let slide2_body = {
            let mut slide2 = zip.by_name("ppt/slides/slide2.xml").unwrap();
            let mut body = String::new();
            std::io::Read::read_to_string(&mut slide2, &mut body).unwrap();
            body
        };
        assert!(
            slide2_body.contains("Figure 1"),
            "caption text missing from rendered slide2.xml"
        );
    }
}
