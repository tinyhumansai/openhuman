use super::*;

#[test]
fn decode_pcm16le_frame_rejects_odd_length() {
    assert!(decode_pcm16le_frame(&[1, 2, 3]).is_none());
}

#[test]
fn decode_pcm16le_frame_decodes_samples() {
    let samples = decode_pcm16le_frame(&[0x01, 0x00, 0xff, 0xff]).expect("decode");
    assert_eq!(samples, vec![1, -1]);
}

#[test]
fn append_stream_samples_keeps_full_audio_and_trims_window() {
    let mut audio = vec![0; MAX_STREAM_BUFFER_SAMPLES - 2];
    let mut full = vec![1, 2];
    let ok = append_stream_samples(&mut audio, &mut full, &[3, 4, 5, 6]);

    assert!(ok, "should succeed when under cap");
    assert_eq!(full, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(audio.len(), MAX_STREAM_BUFFER_SAMPLES);
    assert_eq!(&audio[audio.len() - 4..], &[3, 4, 5, 6]);
}

/// Feed enough samples to hit the full-audio cap and verify:
/// 1. The buffer does NOT grow past `MAX_FULL_AUDIO_SAMPLES`.
/// 2. `append_stream_samples` returns `false` (cap-exceeded signal) when the next
///    chunk would overflow.
#[test]
fn append_stream_samples_enforces_full_audio_cap() {
    let chunk_size = 1_024usize;
    let mut audio = Vec::new();
    let mut full = Vec::new();

    // Fill up to exactly the cap in chunks.
    let full_chunks = MAX_FULL_AUDIO_SAMPLES / chunk_size;
    let chunk = vec![0i16; chunk_size];
    for _ in 0..full_chunks {
        let ok = append_stream_samples(&mut audio, &mut full, &chunk);
        assert!(ok, "should succeed while under cap");
    }

    // The buffer may now be at or just below MAX_FULL_AUDIO_SAMPLES (depending on
    // whether MAX_FULL_AUDIO_SAMPLES is an exact multiple of chunk_size).
    assert!(
        full.len() <= MAX_FULL_AUDIO_SAMPLES,
        "full_audio_buf must not exceed cap before overflow chunk"
    );

    // One more chunk must be rejected.
    let extra = vec![1i16; chunk_size];
    let ok = append_stream_samples(&mut audio, &mut full, &extra);
    assert!(
        !ok,
        "must return false (cap exceeded) when appending would overflow"
    );

    // The buffer must not have grown.
    assert!(
        full.len() <= MAX_FULL_AUDIO_SAMPLES,
        "full_audio_buf must not exceed MAX_FULL_AUDIO_SAMPLES after cap is hit"
    );
}

/// A single oversized chunk that would exceed the cap on its own must also be rejected.
#[test]
fn append_stream_samples_rejects_single_oversized_chunk() {
    let mut audio = Vec::new();
    let mut full = Vec::new();

    // Pre-fill to near the cap (1 sample short).
    let near_full = vec![0i16; MAX_FULL_AUDIO_SAMPLES - 1];
    let ok = append_stream_samples(&mut audio, &mut full, &near_full);
    assert!(ok, "pre-fill should succeed");

    // A 2-sample chunk would push us 1 sample over the cap.
    let ok = append_stream_samples(&mut audio, &mut full, &[7, 8]);
    assert!(!ok, "must return false when chunk crosses the cap boundary");
    assert!(
        full.len() <= MAX_FULL_AUDIO_SAMPLES,
        "full_audio_buf must not exceed cap"
    );
}

#[test]
fn append_stream_samples_returns_false_when_full_audio_cap_reached() {
    let mut audio = vec![];
    let mut full = vec![0i16; MAX_FULL_AUDIO_SAMPLES];
    let ok = append_stream_samples(&mut audio, &mut full, &[1, 2, 3]);

    assert!(!ok, "should return false once cap is reached");
    assert_eq!(
        full.len(),
        MAX_FULL_AUDIO_SAMPLES,
        "full buf must not grow past cap"
    );
    assert!(
        audio.is_empty(),
        "sliding window must not receive new samples"
    );
}

#[test]
fn is_stop_command_only_accepts_stop_type() {
    assert!(is_stop_command(r#"{"type":"stop"}"#));
    assert!(!is_stop_command(r#"{"type":"continue"}"#));
    assert!(!is_stop_command("not json"));
}
