use super::*;

#[tokio::test]
async fn synthesize_piper_rejects_empty_text() {
    let config = Config::default();
    let opts = PiperOptions::default();
    let err = synthesize_piper(&config, "", &opts).await.err().unwrap();
    assert!(err.contains("required"), "empty text must error: {err}");

    let err = synthesize_piper(&config, "   ", &opts).await.err().unwrap();
    assert!(
        err.contains("required"),
        "whitespace text must error: {err}"
    );
}

#[tokio::test]
async fn synthesize_piper_surfaces_binary_lookup_failure() {
    // Make sure a missing PIPER_BIN
    // produces an actionable error, not a panic in the spawn path.
    let prev_piper = std::env::var_os("PIPER_BIN");
    std::env::remove_var("PIPER_BIN");

    let config = Config::default();
    let opts = PiperOptions::default();
    let result = synthesize_piper(&config, "hello world", &opts).await;

    if let Some(v) = prev_piper {
        std::env::set_var("PIPER_BIN", v);
    }

    let err = result.err().expect("missing piper must error");
    assert!(
        err.contains("piper") || err.contains("TTS"),
        "should mention piper or TTS: {err}"
    );
}

#[test]
fn synthetic_viseme_timeline_yields_non_empty_frames() {
    let frames = synthetic_viseme_timeline("hello world");
    assert!(!frames.is_empty(), "must produce at least one frame");
    assert_eq!(frames[0].viseme, "sil", "leading silence");
    assert!(
        frames.last().unwrap().end_ms >= 80,
        "tail frame must extend past the leading silence"
    );
}

#[test]
fn synthetic_viseme_timeline_handles_whitespace_only_text() {
    // Whitespace-only input would normally be rejected upstream, but
    // the helper itself must not panic — defends against a future
    // caller that bypasses the validator.
    let frames = synthetic_viseme_timeline("   ");
    assert!(!frames.is_empty());
    // chars().filter(non-ws).count() is 0 → min 1 → 80 ms total.
    assert_eq!(frames[1].end_ms, 80);
}

#[test]
fn synthetic_viseme_timeline_scales_with_length() {
    let short = synthetic_viseme_timeline("hi");
    let long = synthetic_viseme_timeline("the quick brown fox jumps");
    assert!(
        long.last().unwrap().end_ms > short.last().unwrap().end_ms,
        "longer text should produce a longer timeline"
    );
}
