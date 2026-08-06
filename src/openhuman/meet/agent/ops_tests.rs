//! Additional unit tests for `meet_agent::ops` covering edge cases not
//! exercised by the inline `#[cfg(test)] mod tests` block in `ops.rs`.
//!
//! Wired in via `#[cfg(test)] #[path = "ops_tests.rs"] mod ops_tests;`.

use super::*;

// ── validate_sample_rate ──────────────────────────────────────────────────────

#[test]
fn validate_sample_rate_returns_the_accepted_rate() {
    // The function should echo back REQUIRED_SAMPLE_RATE on success so the
    // caller can use the returned value directly.
    assert_eq!(
        validate_sample_rate(REQUIRED_SAMPLE_RATE).unwrap(),
        REQUIRED_SAMPLE_RATE
    );
}

#[test]
fn validate_sample_rate_error_message_mentions_required_rate() {
    let err = validate_sample_rate(44_100).unwrap_err();
    // The error must mention the unsupported rate and the only allowed rate.
    assert!(
        err.contains("44100") || err.contains("44_100"),
        "error should mention the rejected rate: {err}"
    );
    assert!(
        err.contains(&REQUIRED_SAMPLE_RATE.to_string()),
        "error should mention the required rate: {err}"
    );
}

// ── sanitize_request_id ───────────────────────────────────────────────────────

#[test]
fn sanitize_request_id_trims_whitespace() {
    let result = sanitize_request_id("  abc-123_xyz  ").unwrap();
    assert_eq!(result, "abc-123_xyz");
}

#[test]
fn sanitize_request_id_accepts_exactly_64_chars() {
    // 64 valid chars — right at the limit.
    let id = "a".repeat(64);
    assert_eq!(sanitize_request_id(&id).unwrap().len(), 64);
}

#[test]
fn sanitize_request_id_rejects_65_chars() {
    let id = "a".repeat(65);
    assert!(sanitize_request_id(&id).is_err());
}

#[test]
fn sanitize_request_id_rejects_forbidden_chars() {
    // Spaces, slashes, dots, colons — all must be rejected.
    for bad in &["a b", "a/b", "a.b", "a:b", "a@b", "a!b"] {
        assert!(
            sanitize_request_id(bad).is_err(),
            "expected rejection for {:?}",
            bad
        );
    }
}

#[test]
fn sanitize_request_id_accepts_hyphens_and_underscores() {
    sanitize_request_id("550e8400-e29b-41d4-a716-446655440000").unwrap();
    sanitize_request_id("my_request_id").unwrap();
}

#[test]
fn sanitize_request_id_rejects_empty_after_trim() {
    assert!(sanitize_request_id("   ").is_err());
}

// ── frame_rms ─────────────────────────────────────────────────────────────────

#[test]
fn frame_rms_empty_slice_is_zero() {
    assert_eq!(frame_rms(&[]), 0.0);
}

#[test]
fn frame_rms_full_amplitude_is_near_one() {
    // All samples at i16::MAX should give RMS ≈ 1.0
    let samples = vec![i16::MAX; 1600];
    let rms = frame_rms(&samples);
    assert!(
        rms > 0.99 && rms <= 1.0,
        "full-scale amplitude should give RMS ≈ 1.0, got {rms}"
    );
}

#[test]
fn frame_rms_is_always_non_negative() {
    // Negative PCM values (silence dips) must not produce negative RMS.
    let samples: Vec<i16> = (0..320)
        .map(|i| if i % 2 == 0 { -8000 } else { 8000 })
        .collect();
    assert!(frame_rms(&samples) >= 0.0);
}

#[test]
fn frame_rms_dc_offset_is_computed_correctly() {
    // All samples at the same positive value: RMS = |sample| / i16::MAX
    let val: i16 = 1000;
    let samples = vec![val; 320];
    let expected = val as f32 / i16::MAX as f32;
    let actual = frame_rms(&samples);
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

// ── Vad ───────────────────────────────────────────────────────────────────────

fn loud_frame() -> Vec<i16> {
    (0..1600)
        .map(|i| if i % 2 == 0 { 8000 } else { -8000 })
        .collect()
}

#[test]
fn vad_speech_resets_silence_counter() {
    let mut vad = Vad::new();
    // Start a speech burst.
    vad.feed(&loud_frame());
    // Two silent frames.
    vad.feed(&[0; 320]);
    vad.feed(&[0; 320]);
    // Another speech frame should reset the counter — the next silence run
    // must restart from 0, so we need VAD_HANGOVER_FRAMES more silences.
    assert_eq!(vad.feed(&loud_frame()), VadEvent::Speech);
    // Now provide exactly hangover - 1 silences; should still be Silence.
    for _ in 0..VAD_HANGOVER_FRAMES - 1 {
        let ev = vad.feed(&[0; 320]);
        assert_ne!(ev, VadEvent::EndOfUtterance);
    }
    // One more should trigger EndOfUtterance.
    assert_eq!(vad.feed(&[0; 320]), VadEvent::EndOfUtterance);
}

#[test]
fn vad_consecutive_utterances_are_independent() {
    let mut vad = Vad::new();
    // First utterance + hangover.
    vad.feed(&loud_frame());
    for _ in 0..VAD_HANGOVER_FRAMES {
        vad.feed(&[0; 320]);
    }
    // Start second utterance — must trigger Speech, not Idle.
    assert_eq!(vad.feed(&loud_frame()), VadEvent::Speech);
}

#[test]
fn vad_silence_before_hangover_is_not_end_of_utterance() {
    let mut vad = Vad::new();
    vad.feed(&loud_frame());
    // One frame less than hangover — must not fire EndOfUtterance.
    for _ in 0..VAD_HANGOVER_FRAMES - 1 {
        let ev = vad.feed(&[0; 320]);
        assert_eq!(ev, VadEvent::Silence);
    }
    // Hangover frame fires.
    assert_eq!(vad.feed(&[0; 320]), VadEvent::EndOfUtterance);
}

#[test]
fn vad_idle_emitted_for_multiple_silent_frames_at_start() {
    let mut vad = Vad::new();
    for _ in 0..VAD_HANGOVER_FRAMES * 2 {
        assert_eq!(vad.feed(&[0; 320]), VadEvent::Idle);
    }
}
