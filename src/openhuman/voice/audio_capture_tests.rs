//! Tests for the capture thread.
//!
//! The signal processing that used to live here — downmix, resample, RMS, the
//! silence gate, WAV framing — moved to `tinyvoice`, which carries its unit
//! tests. What is left is the part that is genuinely this file's: converting
//! the device's sample format, which happens in the audio callback and is the
//! one transform that cannot move.

use super::*;
use cpal::{SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfigRange};

#[test]
fn i16_to_f32_normalises_to_unit_range() {
    assert_eq!(i16_to_f32(&[0]), vec![0.0]);
    assert_eq!(i16_to_f32(&[16384]), vec![0.5]);
    assert_eq!(i16_to_f32(&[-32768]), vec![-1.0]);
    // Interleaved frames are converted element-wise, order preserved.
    assert_eq!(i16_to_f32(&[0, 16384, -16384]), vec![0.0, 0.5, -0.5]);
}

#[test]
fn u16_to_f32_centers_on_midscale() {
    // Unsigned PCM is offset-binary: 32768 is silence (0.0), the endpoints map
    // to the extremes. This is the conversion the fallback path was missing.
    assert_eq!(u16_to_f32(&[32768]), vec![0.0]);
    assert_eq!(u16_to_f32(&[49152]), vec![0.5]);
    assert_eq!(u16_to_f32(&[0]), vec![-1.0]);
    assert_eq!(u16_to_f32(&[32768, 49152, 16384]), vec![0.0, 0.5, -0.5]);
}

#[test]
fn find_best_config_prefers_target_rate_and_fewer_channels() {
    let configs = vec![
        SupportedStreamConfigRange::new(
            2,
            SampleRate(8_000),
            SampleRate(48_000),
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        ),
        SupportedStreamConfigRange::new(
            1,
            SampleRate(16_000),
            SampleRate(16_000),
            SupportedBufferSize::Unknown,
            SampleFormat::I16,
        ),
    ];

    let best = find_best_config(configs.into_iter()).expect("best config");
    assert_eq!(best.channels(), 1);
    assert_eq!(best.sample_rate(), SampleRate(TARGET_SAMPLE_RATE));
    assert_eq!(best.sample_format(), SampleFormat::I16);
}

#[test]
fn find_best_config_falls_back_to_max_rate_when_target_missing() {
    let configs = vec![SupportedStreamConfigRange::new(
        1,
        SampleRate(22_050),
        SampleRate(44_100),
        SupportedBufferSize::Unknown,
        SampleFormat::F32,
    )];

    let best = find_best_config(configs.into_iter()).expect("best config");
    assert_eq!(best.sample_rate(), SampleRate(44_100));
}

#[test]
fn find_best_config_errors_when_empty() {
    let err = find_best_config(Vec::<SupportedStreamConfigRange>::new().into_iter())
        .expect_err("empty config list should fail");
    assert!(err.contains("no supported audio input configurations"));
}

#[test]
fn append_capped_bounds_the_raw_buffer() {
    // The silence gate used to run inside the capture callback and drop
    // sustained silence, so an idle microphone cost almost nothing. It gates at
    // finalize now, which is too late to bound what the callback collected — so
    // the cap is what stops a recording that is started and never stopped from
    // growing without limit.
    let buffer = parking_lot::Mutex::new(Vec::new());

    // Fill to just under the cap.
    let chunk = vec![0.5f32; 4096];
    while buffer.lock().len() + chunk.len() <= MAX_RAW_SAMPLES {
        append_capped(&buffer, &chunk);
    }
    let before = buffer.lock().len();
    assert!(before > 0);

    // The chunk that crosses the line is truncated, not dropped whole: a
    // recording that hits the cap keeps its first five minutes.
    append_capped(&buffer, &vec![0.5f32; MAX_RAW_SAMPLES]);
    assert_eq!(buffer.lock().len(), MAX_RAW_SAMPLES);

    // Past the cap, further chunks are ignored rather than reallocating.
    append_capped(&buffer, &chunk);
    assert_eq!(buffer.lock().len(), MAX_RAW_SAMPLES);
}
