//! Pure helpers for the `meet_agent` domain: VAD-style end-of-utterance
//! detection, sample-rate sanity, request_id sanitization. Kept out of
//! `session.rs` so they can be unit-tested without a tokio runtime.

/// Lowest sample rate we will accept. Whisper's training data is at
/// 16kHz; downsampling below that loses too much intelligibility.
pub const MIN_SAMPLE_RATE: u32 = 16_000;
/// Highest sample rate we will accept. CEF's audio handler typically
/// emits 48kHz; the shell must downsample, but we accept up to 48k as
/// a safety net for direct passthrough during development.
pub const MAX_SAMPLE_RATE: u32 = 48_000;

/// Validate a sample rate handed in from the shell.
pub fn validate_sample_rate(hz: u32) -> Result<u32, String> {
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&hz) {
        return Err(format!(
            "sample_rate_hz {hz} out of range [{MIN_SAMPLE_RATE}, {MAX_SAMPLE_RATE}]"
        ));
    }
    Ok(hz)
}

/// Same shape as `meet_call::sanitize_request_id` in the shell — keeping
/// the rule symmetric on both sides means a session key the shell minted
/// is always accepted by core and vice-versa.
pub fn sanitize_request_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("request_id must not be empty".into());
    }
    if trimmed.len() > 64 {
        return Err("request_id exceeds 64 characters".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("request_id contains forbidden characters".into());
    }
    Ok(trimmed.to_string())
}

/// Crude energy-based VAD. Computes RMS over the supplied PCM16LE samples
/// and reports whether they are above a "speech-y" threshold. The brain
/// uses this in combination with a hangover counter to decide when an
/// utterance has ended (see `Vad::feed`).
///
/// Crude on purpose: a real model-based VAD (Silero, webrtcvad) is the
/// follow-up; for the MVP the goal is "did somebody just stop talking
/// for ~600ms?", which RMS handles fine.
pub fn frame_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    let mean = sum_sq / samples.len() as f64;
    (mean.sqrt() / i16::MAX as f64) as f32
}

/// RMS above this is "voice-ish". Picked empirically against
/// `voice::streaming` test fixtures — anything below this is room tone.
pub const VAD_RMS_THRESHOLD: f32 = 0.015;

/// Number of consecutive sub-threshold frames that mean the speaker has
/// stopped. At ~100ms-per-frame (the cadence the shell pushes), 6 frames
/// ≈ 600ms of silence — comfortable end-of-utterance marker without
/// chopping mid-thought.
pub const VAD_HANGOVER_FRAMES: u32 = 6;

/// Stateful VAD wrapper. Owned by the session.
#[derive(Debug, Default)]
pub struct Vad {
    /// True once we've seen at least one speech-y frame for the current
    /// utterance — prevents firing "end of utterance" on a freshly-opened
    /// session that has never seen audio.
    in_utterance: bool,
    /// Consecutive silent frames since the last speech-y one.
    silence_run: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// Speech-y frame; ignore.
    Speech,
    /// Silent frame, but not enough to close the utterance yet.
    Silence,
    /// `VAD_HANGOVER_FRAMES` of silence after speech — turn ends now.
    EndOfUtterance,
    /// Silence with no preceding speech this session — caller can skip
    /// any buffer-flush work.
    Idle,
}

impl Vad {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a single PCM frame and learn whether it ended the utterance.
    pub fn feed(&mut self, samples: &[i16]) -> VadEvent {
        let rms = frame_rms(samples);
        if rms >= VAD_RMS_THRESHOLD {
            self.in_utterance = true;
            self.silence_run = 0;
            VadEvent::Speech
        } else if !self.in_utterance {
            VadEvent::Idle
        } else {
            self.silence_run += 1;
            if self.silence_run >= VAD_HANGOVER_FRAMES {
                self.in_utterance = false;
                self.silence_run = 0;
                VadEvent::EndOfUtterance
            } else {
                VadEvent::Silence
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_sample_rate_accepts_common_rates() {
        validate_sample_rate(16_000).unwrap();
        validate_sample_rate(48_000).unwrap();
    }

    #[test]
    fn validate_sample_rate_rejects_out_of_range() {
        assert!(validate_sample_rate(8_000).is_err());
        assert!(validate_sample_rate(96_000).is_err());
    }

    #[test]
    fn sanitize_request_id_matches_shell_rules() {
        sanitize_request_id("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(sanitize_request_id("").is_err());
        assert!(sanitize_request_id("a/b").is_err());
        assert!(sanitize_request_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn frame_rms_is_zero_for_silence() {
        assert_eq!(frame_rms(&[0; 320]), 0.0);
    }

    #[test]
    fn frame_rms_grows_with_amplitude() {
        let quiet: Vec<i16> = (0..320).map(|i| if i % 2 == 0 { 1000i16 } else { -1000 }).collect();
        let loud: Vec<i16> = (0..320).map(|i| if i % 2 == 0 { 8000i16 } else { -8000 }).collect();
        assert!(frame_rms(&loud) > frame_rms(&quiet));
    }

    /// Build a frame that's deterministically above the VAD threshold.
    fn loud_frame() -> Vec<i16> {
        // Half-amplitude square wave — comfortably above VAD_RMS_THRESHOLD
        // without saturating clamps in downstream tests.
        (0..1600)
            .map(|i| if i % 2 == 0 { 8000 } else { -8000 })
            .collect()
    }

    #[test]
    fn vad_idle_until_first_speech() {
        let mut vad = Vad::new();
        for _ in 0..10 {
            assert_eq!(vad.feed(&[0; 320]), VadEvent::Idle);
        }
    }

    #[test]
    fn vad_emits_end_of_utterance_after_hangover() {
        let mut vad = Vad::new();
        assert_eq!(vad.feed(&loud_frame()), VadEvent::Speech);
        for i in 0..VAD_HANGOVER_FRAMES - 1 {
            assert_eq!(
                vad.feed(&[0; 320]),
                VadEvent::Silence,
                "frame #{i} of silence run"
            );
        }
        assert_eq!(vad.feed(&[0; 320]), VadEvent::EndOfUtterance);
    }

    #[test]
    fn vad_resets_after_utterance() {
        let mut vad = Vad::new();
        vad.feed(&loud_frame());
        for _ in 0..VAD_HANGOVER_FRAMES {
            vad.feed(&[0; 320]);
        }
        // Next silent frame after end-of-utterance should be Idle, not
        // a fresh Silence run.
        assert_eq!(vad.feed(&[0; 320]), VadEvent::Idle);
    }
}
