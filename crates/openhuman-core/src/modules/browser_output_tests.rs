use super::*;

fn held(bytes: &[u8]) -> OutputRef {
    use sha2::{Digest, Sha256};
    OutputRef {
        id: "o-1".into(),
        total_bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        media_type: "image/png".into(),
        width: 1,
        height: 1,
    }
}

fn chunk(offset: u64, bytes: &[u8], eof: bool) -> OutputChunk {
    use base64::Engine as _;
    OutputChunk {
        id: "o-1".into(),
        offset,
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        eof,
    }
}

#[test]
fn a_screenshot_is_reassembled_from_its_chunks_and_checked() {
    let image = b"\x89PNG\r\n\x1a\nsome pixels";
    let handle = held(image);
    let chunks = [chunk(0, &image[..5], false), chunk(5, &image[5..], true)];
    assert_eq!(assemble_output(&handle, &chunks, true).unwrap(), image);
    // Partway, only what came so far.
    assert_eq!(
        assemble_output(&handle, &chunks[..1], false).unwrap(),
        &image[..5]
    );
}

#[test]
fn a_screenshot_that_does_not_match_its_handle_is_refused() {
    let image = b"\x89PNG\r\n\x1a\nsome pixels";
    let handle = held(image);
    // Short, out of order, altered, or bigger than any screenshot.
    assert!(assemble_output(&handle, &[chunk(0, &image[..5], true)], true).is_err());
    assert!(assemble_output(&handle, &[chunk(3, image, true)], true).is_err());
    let mut altered = image.to_vec();
    altered[9] ^= 1;
    assert!(assemble_output(&handle, &[chunk(0, &altered, true)], true).is_err());
    let mut huge = held(image);
    huge.total_bytes = MAX_SCREENSHOT_BYTES + 1;
    assert!(assemble_output(&huge, &[chunk(0, image, true)], true).is_err());
}
