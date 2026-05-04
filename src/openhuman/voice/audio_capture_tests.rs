use super::*;
use cpal::{SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfigRange};

#[test]
fn to_mono_passthrough_single_channel() {
    let input = vec![0.1, 0.2, 0.3];
    assert_eq!(to_mono(&input, 1), input);
}

#[test]
fn to_mono_averages_stereo() {
    let input = vec![0.0, 1.0, 0.5, 0.5];
    let mono = to_mono(&input, 2);
    assert_eq!(mono.len(), 2);
    assert!((mono[0] - 0.5).abs() < 1e-6);
    assert!((mono[1] - 0.5).abs() < 1e-6);
}

#[test]
fn to_mono_averages_multichannel_frames() {
    let input = vec![0.0, 0.5, 1.0, 0.25, 0.25, 0.25];
    let mono = to_mono(&input, 3);
    assert_eq!(mono, vec![0.5, 0.25]);
}

#[test]
fn resample_same_rate_passthrough() {
    let input = vec![0.1, 0.2, 0.3];
    let output = resample(&input, TARGET_SAMPLE_RATE);
    assert_eq!(output, input);
}

#[test]
fn resample_downsamples() {
    // 32kHz -> 16kHz should roughly halve the samples.
    let input: Vec<f32> = (0..3200).map(|i| (i as f32 / 3200.0).sin()).collect();
    let output = resample(&input, 32_000);
    // Should be approximately 1600 samples.
    assert!(output.len() >= 1590 && output.len() <= 1610);
}

#[test]
fn resample_upsamples() {
    let input = vec![0.0, 1.0, 0.0, -1.0];
    let output = resample(&input, 8_000);
    assert_eq!(output.len(), 8);
    assert!((output[0] - 0.0).abs() < 1e-6);
    assert!((output[1] - 0.5).abs() < 1e-6);
    assert!((output[2] - 1.0).abs() < 1e-6);
}

#[test]
fn chunk_rms_handles_empty_and_signal() {
    assert_eq!(chunk_rms(&[]), 0.0);
    let rms = chunk_rms(&[1.0, -1.0, 1.0, -1.0]);
    assert!((rms - 1.0).abs() < 1e-6);
}

#[test]
fn finalize_produces_valid_wav() {
    let samples: Vec<f32> = (0..16000)
        .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin())
        .collect();
    let result = finalize_recording(samples, 16_000, 0.5).unwrap();
    assert!(result.wav_bytes.len() > 44); // WAV header is 44 bytes
    assert!((result.duration_secs - 1.0).abs() < 0.1);
    // Check WAV magic bytes.
    assert_eq!(&result.wav_bytes[..4], b"RIFF");
}

#[test]
fn finalize_empty_samples_errors() {
    let result = finalize_recording(vec![], 16_000, 0.0);
    assert!(result.is_err());
}

#[test]
fn update_peak_rms_tracks_maximum() {
    let peak = std::sync::atomic::AtomicU32::new(0);
    // First chunk: low energy
    update_peak_rms(&peak, &[0.01, -0.01, 0.01]);
    let first = f32::from_bits(peak.load(Ordering::Relaxed));
    // Second chunk: higher energy
    update_peak_rms(&peak, &[0.5, -0.5, 0.5]);
    let second = f32::from_bits(peak.load(Ordering::Relaxed));
    assert!(second > first);
    // Third chunk: lower energy — peak should not decrease
    update_peak_rms(&peak, &[0.01, -0.01]);
    let third = f32::from_bits(peak.load(Ordering::Relaxed));
    assert!((third - second).abs() < 1e-6);
}

#[test]
fn update_peak_rms_empty_is_noop() {
    let peak = std::sync::atomic::AtomicU32::new(0.1f32.to_bits());
    update_peak_rms(&peak, &[]);
    assert!((f32::from_bits(peak.load(Ordering::Relaxed)) - 0.1).abs() < 1e-6);
}

#[test]
fn silence_gate_keeps_audio_before_threshold() {
    let mut gate = SilenceGate::new(16_000);
    let near_silent = vec![0.0; 4_000];
    let out = gate.process(&near_silent);
    assert_eq!(out.len(), near_silent.len());
    assert!(!gate.gating);
}

#[test]
fn silence_gate_drops_sustained_silence_and_flushes_on_speech() {
    let mut gate = SilenceGate::new(16_000);
    let silence = vec![0.0; 4_000];

    assert_eq!(gate.process(&silence).len(), silence.len());
    assert!(gate.process(&silence).is_empty());
    assert!(gate.gating);
    assert_eq!(gate.lookahead.len(), 1_600);

    let speech = vec![0.5; 160];
    let out = gate.process(&speech);
    assert_eq!(out.len(), 1_600 + 160);
    assert!(!gate.gating);
    assert!(gate.lookahead.is_empty());
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
