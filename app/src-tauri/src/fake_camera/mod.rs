//! Mascot-as-webcam pipeline.
//!
//! Once at app startup we rasterize the OpenHuman mascot SVG into a
//! 640×480 RGBA bitmap, convert it to YUV420, and write a single-frame
//! YUV4MPEG2 (Y4M) file to the per-user data directory. The file is
//! cached across launches keyed by source-SVG hash so subsequent boots
//! skip the rasterization.
//!
//! At browser launch, `lib.rs` passes the cached path to CEF via
//! `--use-file-for-fake-video-capture=<path>`. CEF reads it on every
//! `getUserMedia({video:true})` call and loops on EOF, so a single
//! frame produces a steady-state still image as the agent's "webcam".
//!
//! No JS is injected anywhere — this is a process-level Chromium flag,
//! not page-level instrumentation.

use std::fs;
use std::path::{Path, PathBuf};

use resvg::usvg::{Options as UsvgOptions, Tree as UsvgTree};
use tiny_skia::{Pixmap, Transform};

/// Output webcam resolution. 640×480 is what every videoconferencing
/// app expects to negotiate against; Meet downscales to whatever it
/// wants from there.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
const FRAMERATE: &str = "F30:1";

/// Mascot SVG embedded at build time. The remotion bundle owns the
/// canonical asset; we vendor a copy of its content via `include_str!`
/// so the shell builds without needing the remotion tree at runtime.
const MASCOT_SVG: &str = include_str!("../../../../remotion/public/mascot.svg");

/// Top-level entrypoint. Returns the path to a Y4M file CEF can read,
/// rasterizing the mascot if no cached version exists.
///
/// Errors are logged + returned as `String` so the caller (lib.rs)
/// can decide whether to skip the fake-camera flag and let the user
/// see the default "no camera" path. We do **not** panic — a missing
/// fake camera is degraded but not fatal.
pub fn ensure_mascot_y4m(data_dir: &Path) -> Result<PathBuf, String> {
    let cache_dir = data_dir.join("cache").join("fake_camera");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;

    let svg_hash = stable_hash(MASCOT_SVG);
    let y4m_path = cache_dir.join(format!("mascot-{WIDTH}x{HEIGHT}-{svg_hash:016x}.y4m"));

    if y4m_path.exists() {
        log::info!(
            "[fake-camera] reusing cached mascot Y4M path={}",
            y4m_path.display()
        );
        return Ok(y4m_path);
    }

    log::info!(
        "[fake-camera] rasterizing mascot {}x{} -> {}",
        WIDTH,
        HEIGHT,
        y4m_path.display()
    );
    let rgba = rasterize_svg(MASCOT_SVG)?;
    let y4m_bytes = encode_single_frame_y4m(&rgba);

    // Atomic-ish write: write to .partial then rename, so a crash
    // mid-write never leaves CEF reading a half-finished Y4M.
    let tmp_path = y4m_path.with_extension("y4m.partial");
    fs::write(&tmp_path, &y4m_bytes).map_err(|e| format!("write y4m: {e}"))?;
    fs::rename(&tmp_path, &y4m_path).map_err(|e| format!("rename y4m: {e}"))?;
    Ok(y4m_path)
}

/// Render the SVG to a 640×480 RGBA8 bitmap, letterboxed onto a flat
/// background so the mascot looks centered in the participant tile
/// regardless of source aspect ratio.
fn rasterize_svg(svg: &str) -> Result<Vec<u8>, String> {
    let tree =
        UsvgTree::from_str(svg, &UsvgOptions::default()).map_err(|e| format!("parse svg: {e}"))?;
    let svg_size = tree.size();
    let svg_w = svg_size.width();
    let svg_h = svg_size.height();
    if svg_w <= 0.0 || svg_h <= 0.0 {
        return Err("mascot svg has zero size".into());
    }

    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).ok_or_else(|| "alloc pixmap".to_string())?;
    // Background fill — Meet's tile is rectangular and we want a clean
    // backdrop, not transparent (which the YUV conversion would
    // collapse to black anyway).
    pixmap.fill(tiny_skia::Color::from_rgba8(247, 244, 238, 255));

    // Fit the mascot inside the frame with a 12% margin so it doesn't
    // get cropped at the corners by Meet's rounded mask.
    let margin = 0.12;
    let target_w = (WIDTH as f32) * (1.0 - 2.0 * margin);
    let target_h = (HEIGHT as f32) * (1.0 - 2.0 * margin);
    let scale = (target_w / svg_w).min(target_h / svg_h);
    let drawn_w = svg_w * scale;
    let drawn_h = svg_h * scale;
    let tx = ((WIDTH as f32) - drawn_w) / 2.0;
    let ty = ((HEIGHT as f32) - drawn_h) / 2.0;

    let transform = Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(pixmap.take())
}

/// Convert an RGBA8 buffer (length WIDTH * HEIGHT * 4) to a Y4M file
/// containing a single FRAME using BT.601 limited-range coefficients.
/// Chromium's fake video capture re-reads the file in a loop, so one
/// frame is enough for a steady image.
fn encode_single_frame_y4m(rgba: &[u8]) -> Vec<u8> {
    let header = format!(
        "YUV4MPEG2 W{WIDTH} H{HEIGHT} {FRAMERATE} Ip A1:1 C420jpeg Xopenhuman-mascot\nFRAME\n"
    );

    let pixel_count = (WIDTH * HEIGHT) as usize;
    let mut y_plane = Vec::with_capacity(pixel_count);
    let chroma_count = ((WIDTH / 2) * (HEIGHT / 2)) as usize;
    let mut u_plane = Vec::with_capacity(chroma_count);
    let mut v_plane = Vec::with_capacity(chroma_count);

    // Y plane: per-pixel luma.
    for chunk in rgba.chunks_exact(4) {
        let (r, g, b) = (chunk[0] as f32, chunk[1] as f32, chunk[2] as f32);
        let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
        y_plane.push(y);
    }

    // U/V planes: average each 2×2 block.
    for by in (0..HEIGHT).step_by(2) {
        for bx in (0..WIDTH).step_by(2) {
            let mut r_sum = 0.0;
            let mut g_sum = 0.0;
            let mut b_sum = 0.0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let x = bx + dx;
                    let y = by + dy;
                    let idx = ((y * WIDTH + x) * 4) as usize;
                    r_sum += rgba[idx] as f32;
                    g_sum += rgba[idx + 1] as f32;
                    b_sum += rgba[idx + 2] as f32;
                }
            }
            let r = r_sum / 4.0;
            let g = g_sum / 4.0;
            let b = b_sum / 4.0;
            let u = (-0.169 * r - 0.331 * g + 0.5 * b + 128.0).clamp(0.0, 255.0) as u8;
            let v = (0.5 * r - 0.419 * g - 0.081 * b + 128.0).clamp(0.0, 255.0) as u8;
            u_plane.push(u);
            v_plane.push(v);
        }
    }

    let mut out = Vec::with_capacity(header.len() + y_plane.len() + u_plane.len() + v_plane.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(&y_plane);
    out.extend_from_slice(&u_plane);
    out.extend_from_slice(&v_plane);
    out
}

/// Stable, deterministic hash of a string — used to key the Y4M cache
/// against the source SVG. We don't need cryptographic strength, just
/// "did the SVG change?", so std's `DefaultHasher` is fine.
fn stable_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn y4m_header_includes_dimensions_and_colorspace() {
        let dummy = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        let bytes = encode_single_frame_y4m(&dummy);
        let header_end = bytes.iter().position(|&b| b == b'\n').unwrap();
        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
        assert!(header.contains(&format!("W{WIDTH}")));
        assert!(header.contains(&format!("H{HEIGHT}")));
        assert!(header.contains("C420jpeg"));
    }

    #[test]
    fn y4m_payload_size_matches_yuv420_layout() {
        let dummy = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        let bytes = encode_single_frame_y4m(&dummy);
        // Header up to first newline, then "FRAME\n", then planes.
        let frame_marker = b"FRAME\n";
        let frame_idx = bytes
            .windows(frame_marker.len())
            .position(|w| w == frame_marker)
            .expect("FRAME marker present");
        let payload_len = bytes.len() - frame_idx - frame_marker.len();
        let expected = (WIDTH * HEIGHT) as usize + 2 * ((WIDTH / 2) * (HEIGHT / 2)) as usize;
        assert_eq!(payload_len, expected);
    }

    #[test]
    fn rasterize_svg_produces_correctly_sized_buffer() {
        let rgba = rasterize_svg(MASCOT_SVG).expect("rasterize");
        assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
    }

    #[test]
    fn stable_hash_is_deterministic() {
        assert_eq!(stable_hash("openhuman"), stable_hash("openhuman"));
        assert_ne!(stable_hash("a"), stable_hash("b"));
    }
}
