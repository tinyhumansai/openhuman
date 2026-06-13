//! Self-contained PNG / JPEG sniffing + pixel-dimension extraction for
//! the presentation image pipeline.
//!
//! This deliberately does **not** reuse `agent::multimodal`'s private
//! magic-byte helpers, for three reasons:
//!
//! 1. Those helpers live outside this feature's edit boundary.
//! 2. They accept `webp` / `gif` / `bmp`, none of which `ppt-rs` 0.2.14
//!    can embed safely — there is no `webp` default in the generated
//!    `[Content_Types].xml`, and `ImageBuilder::auto` misclassifies
//!    `webp` as `PNG`, producing a part PowerPoint refuses to render.
//!    v1 therefore restricts embeddable images to PNG + JPEG.
//! 3. They do not expose pixel dimensions, which we need to place
//!    images aspect-correctly in the single-column layout.

/// Return the `ppt-rs` format token (`"PNG"` / `"JPEG"`) for `bytes`,
/// or `None` if the bytes are not one of the two embeddable formats.
pub(super) fn sniff_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("PNG")
    } else if bytes.len() >= 3 && bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("JPEG")
    } else {
        None
    }
}

/// Native `(width, height)` in pixels for a PNG or JPEG. Returns `None`
/// when the header is truncated / malformed or the format is unsupported.
pub(super) fn pixel_dimensions(bytes: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "PNG" => png_dimensions(bytes),
        "JPEG" => jpeg_dimensions(bytes),
        _ => None,
    }
}

/// PNG: 8-byte signature, then an `IHDR` chunk whose width / height are
/// big-endian `u32`s at byte offsets 16 and 20.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// JPEG: walk the marker segments until a Start-Of-Frame (`SOF0`/`SOF2`,
/// and the other non-differential / progressive SOF markers) is hit; its
/// payload carries height then width as big-endian `u16`s.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // skip the leading FF D8 SOI
    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        i += 2;
        // Standalone markers (no length field): padding fill bytes and
        // RSTn / SOI / EOI. Skip without consuming a segment length.
        if marker == 0xFF || marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if i + 1 >= bytes.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if seg_len < 2 {
            return None;
        }
        // SOF markers carrying frame dimensions. Excludes 0xC4 (DHT),
        // 0xC8 (JPG), 0xCC (DAC), which share the 0xCn range but are not
        // frame headers.
        let is_sof = matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        );
        if is_sof {
            // segment: [len_hi len_lo precision h_hi h_lo w_hi w_lo ...]
            if i + 6 >= bytes.len() {
                return None;
            }
            let h = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            if w == 0 || h == 0 {
                return None;
            }
            return Some((w, h));
        }
        i += seg_len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical 1×1 PNG (full IHDR + IDAT + IEND).
    fn png_1x1() -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")
            .unwrap()
    }

    /// Minimal JPEG: SOI + APP0 stub + SOF0 declaring 7×5.
    fn jpeg_7x5() -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, // APP0, len=4, 2 payload bytes
            0xFF, 0xC0, 0x00, 0x0B, // SOF0, len=11
            0x08, // precision
            0x00, 0x05, // height = 5
            0x00, 0x07, // width = 7
            0x03, 0x00, 0x00, 0x00, // components (filler)
            0xFF, 0xD9, // EOI
        ]
    }

    #[test]
    fn sniffs_png_and_jpeg() {
        assert_eq!(sniff_format(&png_1x1()), Some("PNG"));
        assert_eq!(sniff_format(&jpeg_7x5()), Some("JPEG"));
    }

    #[test]
    fn rejects_non_image_and_unsupported() {
        assert_eq!(sniff_format(b"not an image"), None);
        // GIF magic — recognised by multimodal but NOT embeddable here.
        assert_eq!(sniff_format(b"GIF89a....."), None);
        // WebP magic — same story.
        assert_eq!(sniff_format(b"RIFF\0\0\0\0WEBP"), None);
    }

    #[test]
    fn reads_png_dimensions() {
        assert_eq!(pixel_dimensions(&png_1x1(), "PNG"), Some((1, 1)));
    }

    #[test]
    fn reads_jpeg_dimensions() {
        assert_eq!(pixel_dimensions(&jpeg_7x5(), "JPEG"), Some((7, 5)));
    }

    #[test]
    fn truncated_headers_yield_none() {
        assert_eq!(png_dimensions(&[0x89, 0x50, 0x4E, 0x47]), None);
        assert_eq!(jpeg_dimensions(&[0xFF, 0xD8]), None);
    }
}
