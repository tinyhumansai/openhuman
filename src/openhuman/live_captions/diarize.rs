//! Speaker diarization for live captions.
//!
//! Provides energy-based speaker change detection using spectral centroid
//! and zero-crossing rate features. Assigns speaker labels (Speaker_0,
//! Speaker_1, etc.) to audio segments.
//!
//! ## Approach
//!
//! Uses a sliding-window feature extractor that computes:
//! - RMS energy
//! - Zero-crossing rate (ZCR)
//! - Spectral centroid approximation
//!
//! Speaker changes are detected when the feature distance between consecutive
//! windows exceeds a threshold. This is a lightweight CPU-only approach that
//! works without ML models — suitable for Phase 1.
//!
//! For production accuracy, integrate `polyvoice` crate (ECAPA-TDNN embeddings
//! + K-means clustering) in a follow-up.

use tracing::debug;

const LOG_PREFIX: &str = "[live-captions-diarize]";

/// Window size for feature extraction: 500ms @ 16kHz.
const WINDOW_SAMPLES: usize = 8_000;
/// Hop size: 250ms.
const HOP_SAMPLES: usize = 4_000;
/// Threshold for speaker change detection (empirically tuned).
const CHANGE_THRESHOLD: f64 = 0.35;

/// A detected speaker segment.
#[derive(Debug, Clone)]
pub struct SpeakerSegment {
    pub speaker: String,
    pub start_sample: usize,
    pub end_sample: usize,
}

/// Audio features for a single window.
#[derive(Debug, Clone)]
struct WindowFeatures {
    rms: f64,
    zcr: f64,
    centroid: f64,
}

/// Perform speaker diarization on PCM16LE audio @ 16kHz.
/// Returns a list of speaker segments with labels.
pub fn diarize(pcm: &[i16], sample_rate: u32) -> Vec<SpeakerSegment> {
    if sample_rate != 16_000 {
        debug!(
            "{} unsupported sample_rate={}, returning single segment",
            LOG_PREFIX, sample_rate
        );
        return vec![SpeakerSegment {
            speaker: "Speaker_0".into(),
            start_sample: 0,
            end_sample: pcm.len(),
        }];
    }
    if pcm.len() < WINDOW_SAMPLES {
        return vec![SpeakerSegment {
            speaker: "Speaker_0".into(),
            start_sample: 0,
            end_sample: pcm.len(),
        }];
    }

    let features = extract_features(pcm);
    if features.is_empty() {
        return vec![SpeakerSegment {
            speaker: "Speaker_0".into(),
            start_sample: 0,
            end_sample: pcm.len(),
        }];
    }

    // Detect speaker changes by comparing consecutive feature windows.
    let mut segments: Vec<SpeakerSegment> = Vec::new();
    let mut current_speaker = 0u32;
    let mut segment_start = 0usize;

    for i in 1..features.len() {
        let dist = feature_distance(&features[i - 1], &features[i]);
        if dist > CHANGE_THRESHOLD {
            // Try to identify speaker from voice profile before assigning generic label.
            let seg_audio = &pcm[segment_start..(i * HOP_SAMPLES).min(pcm.len())];
            let speaker_label = match super::voice_profiles::identify_speaker(seg_audio, 0.7) {
                Some((_, name, _)) => name,
                None => format!("Speaker_{current_speaker}"),
            };
            segments.push(SpeakerSegment {
                speaker: speaker_label,
                start_sample: segment_start,
                end_sample: i * HOP_SAMPLES,
            });
            segment_start = i * HOP_SAMPLES;
            current_speaker = (current_speaker + 1) % 10; // Max 10 speakers
        }
    }

    // Final segment — also try voice profile identification.
    let final_audio = &pcm[segment_start..];
    let final_label = match super::voice_profiles::identify_speaker(final_audio, 0.7) {
        Some((_, name, _)) => name,
        None => format!("Speaker_{current_speaker}"),
    };
    segments.push(SpeakerSegment {
        speaker: final_label,
        start_sample: segment_start,
        end_sample: pcm.len(),
    });

    debug!(
        "{LOG_PREFIX} diarized {} samples into {} segments ({} speakers) sr={sample_rate}",
        pcm.len(),
        segments.len(),
        segments
            .iter()
            .map(|s| &s.speaker)
            .collect::<std::collections::HashSet<_>>()
            .len()
    );

    segments
}

/// Convert sample offset to milliseconds.
pub fn samples_to_ms(samples: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    (samples as u64 * 1000) / sample_rate as u64
}

fn extract_features(pcm: &[i16]) -> Vec<WindowFeatures> {
    let mut features = Vec::new();
    let mut offset = 0;
    while offset + WINDOW_SAMPLES <= pcm.len() {
        let window = &pcm[offset..offset + WINDOW_SAMPLES];
        features.push(compute_window_features(window));
        offset += HOP_SAMPLES;
    }
    features
}

fn compute_window_features(window: &[i16]) -> WindowFeatures {
    let n = window.len() as f64;

    // RMS energy.
    let rms = (window.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / n).sqrt();

    // Zero-crossing rate.
    let zcr = window
        .windows(2)
        .filter(|w| (w[0] >= 0) != (w[1] >= 0))
        .count() as f64
        / (n - 1.0);

    // Spectral centroid approximation (using magnitude-weighted frequency bins).
    // Simplified: use the weighted average of absolute sample differences.
    let total_energy: f64 = window.iter().map(|&s| (s as f64).abs()).sum();
    let centroid = if total_energy > 0.0 {
        window
            .iter()
            .enumerate()
            .map(|(i, &s)| i as f64 * (s as f64).abs())
            .sum::<f64>()
            / total_energy
    } else {
        0.0
    };

    // Normalize centroid to [0, 1].
    let centroid_norm = centroid / n;

    WindowFeatures {
        rms: rms / 32768.0, // Normalize to [0, 1]
        zcr,
        centroid: centroid_norm,
    }
}

/// Euclidean distance between two feature vectors (normalized).
fn feature_distance(a: &WindowFeatures, b: &WindowFeatures) -> f64 {
    let dr = a.rms - b.rms;
    let dz = a.zcr - b.zcr;
    let dc = a.centroid - b.centroid;
    (dr * dr + dz * dz + dc * dc).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diarize_short_audio_single_speaker() {
        let pcm = vec![0i16; 1000]; // Too short for windowing
        let segments = diarize(&pcm, 16_000);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].speaker, "Speaker_0");
    }

    #[test]
    fn diarize_silence_single_speaker() {
        let pcm = vec![0i16; 32_000]; // 2 seconds of silence
        let segments = diarize(&pcm, 16_000);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].speaker, "Speaker_0");
    }

    #[test]
    fn diarize_detects_speaker_change() {
        // Create audio with a clear energy change (simulating speaker switch).
        let mut pcm = vec![0i16; 16_000]; // 1s silence
        pcm.extend(vec![10_000i16; 16_000]); // 1s loud
        pcm.extend(vec![0i16; 16_000]); // 1s silence again
        let segments = diarize(&pcm, 16_000);
        // Should detect at least one speaker change.
        assert!(segments.len() >= 2);
    }

    #[test]
    fn samples_to_ms_conversion() {
        assert_eq!(samples_to_ms(16_000, 16_000), 1000);
        assert_eq!(samples_to_ms(8_000, 16_000), 500);
        assert_eq!(samples_to_ms(0, 16_000), 0);
    }

    #[test]
    fn feature_distance_identical_is_zero() {
        let f = WindowFeatures {
            rms: 0.5,
            zcr: 0.3,
            centroid: 0.4,
        };
        assert_eq!(feature_distance(&f, &f), 0.0);
    }
}
