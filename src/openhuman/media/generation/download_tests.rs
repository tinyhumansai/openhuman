use super::*;

#[test]
fn extension_prefers_content_type() {
    assert_eq!(
        extension_for("image", Some("image/png"), "https://x/y"),
        "png"
    );
    assert_eq!(
        extension_for("image", Some("image/webp"), "https://x/y"),
        "webp"
    );
    assert_eq!(
        extension_for("video", Some("video/mp4"), "https://x/y"),
        "mp4"
    );
}

#[test]
fn extension_falls_back_to_url_then_kind() {
    assert_eq!(
        extension_for("image", None, "https://x/y/a.webp?sig=1"),
        "webp"
    );
    assert_eq!(extension_for("video", None, "https://x/y/clip"), "mp4");
    assert_eq!(extension_for("image", None, "https://x/y/clip"), "png");
}
