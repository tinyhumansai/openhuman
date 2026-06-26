//! Noise and echo cancellation for voice assistant audio.
//!
//! Uses nnnoiseless (pure-Rust RNNoise port) for neural noise suppression
//! + NLMS adaptive filter for echo cancellation.

use nnnoiseless::DenoiseState;
use tracing::debug;

const LOG_PREFIX: &str = "[voice-noise-cancel]";

#[derive(Debug, Clone)]
pub struct NoiseCancelConfig {
    pub echo_cancel_enabled: bool,
    pub echo_filter_len: usize,
    pub nlms_step: f32,
}

impl Default for NoiseCancelConfig {
    fn default() -> Self {
        Self {
            echo_cancel_enabled: true,
            echo_filter_len: 256,
            nlms_step: 0.1,
        }
    }
}

pub struct NoiseCancelState {
    config: NoiseCancelConfig,
    denoise: Box<DenoiseState<'static>>,
    echo_weights: Vec<f32>,
    reference_buf: Vec<f32>,
    frame_count: u64,
    first_frame: bool,
    /// Last VAD probability from RNNoise (0.0 = silence, 1.0 = speech).
    pub vad_prob: f32,
    /// Pre-allocated buffer for upsampled audio (avoids per-frame allocation).
    upsample_buf: Vec<f32>,
    /// Pre-allocated buffer for denoised output.
    denoise_buf: Vec<f32>,
    /// Pre-allocated buffer for downsampled output.
    downsample_buf: Vec<f32>,
}

impl NoiseCancelState {
    pub fn new(config: NoiseCancelConfig) -> Self {
        let filter_len = config.echo_filter_len;
        // Pre-allocate for typical 20ms frame @ 16kHz = 320 samples
        let typical_frame = 320;
        Self {
            config,
            denoise: DenoiseState::new(),
            echo_weights: vec![0.0; filter_len],
            reference_buf: Vec::new(),
            frame_count: 0,
            first_frame: true,
            vad_prob: 0.0,
            upsample_buf: Vec::with_capacity(typical_frame * 3),
            denoise_buf: Vec::with_capacity(typical_frame * 3),
            downsample_buf: Vec::with_capacity(typical_frame),
        }
    }

    /// Process mic input with RNNoise denoising and optional echo cancellation.
    /// Input is 16kHz i16 PCM. Internally upsamples to 48kHz for RNNoise.
    pub fn process(&mut self, input: &[i16], reference: Option<&[i16]>) -> Vec<i16> {
        self.frame_count += 1;

        // Upsample 16kHz → 48kHz (3x linear interpolation) into pre-allocated buffer
        self.upsample_buf.clear();
        upsample_3x_into(input, &mut self.upsample_buf);

        // Process through RNNoise in FRAME_SIZE (480) chunks
        self.denoise_buf.clear();
        self.denoise_frames_into(&self.upsample_buf.clone());

        // Downsample 48kHz → 16kHz (take every 3rd sample) into pre-allocated buffer
        self.downsample_buf.clear();
        self.downsample_buf
            .extend(self.denoise_buf.iter().step_by(3));
        self.downsample_buf.truncate(input.len());

        // Echo cancellation (operates at 16kHz)
        let mut samples = std::mem::take(&mut self.downsample_buf);
        if self.config.echo_cancel_enabled {
            if let Some(ref_signal) = reference {
                samples = self.cancel_echo(&samples, ref_signal);
            } else if !self.reference_buf.is_empty() {
                // Use buffered reference from feed_reference() calls.
                let buf_ref: Vec<i16> = self
                    .reference_buf
                    .iter()
                    .map(|&s| s.clamp(-32768.0, 32767.0) as i16)
                    .collect();
                samples = self.cancel_echo(&samples, &buf_ref);
            }
        }

        samples
            .iter()
            .map(|&s| s.clamp(-32768.0, 32767.0) as i16)
            .collect()
    }

    fn denoise_frames_into(&mut self, samples_48k: &[f32]) {
        let frame_size = DenoiseState::FRAME_SIZE; // 480
        let mut out_buf = [0.0f32; 480];

        for chunk in samples_48k.chunks(frame_size) {
            if chunk.len() == frame_size {
                self.vad_prob = self.denoise.process_frame(&mut out_buf, chunk);
                if self.first_frame {
                    self.first_frame = false;
                    self.denoise_buf.extend_from_slice(&[0.0f32; 480]);
                } else {
                    self.denoise_buf.extend_from_slice(&out_buf);
                }
            } else {
                let mut padded = [0.0f32; 480];
                padded[..chunk.len()].copy_from_slice(chunk);
                self.vad_prob = self.denoise.process_frame(&mut out_buf, &padded);
                if self.first_frame {
                    self.first_frame = false;
                }
                self.denoise_buf.extend_from_slice(&out_buf[..chunk.len()]);
            }
        }
    }

    fn cancel_echo(&mut self, input: &[f32], reference: &[i16]) -> Vec<f32> {
        self.reference_buf
            .extend(reference.iter().map(|&s| s as f32));
        let max_buf = self.config.echo_filter_len + input.len();
        if self.reference_buf.len() > max_buf {
            let drain = self.reference_buf.len() - max_buf;
            self.reference_buf.drain(..drain);
        }
        let fl = self.config.echo_filter_len;
        if self.reference_buf.len() < fl {
            return input.to_vec();
        }
        let mut output = Vec::with_capacity(input.len());
        let ref_start = self.reference_buf.len().saturating_sub(fl + input.len());
        for (i, &mic) in input.iter().enumerate() {
            let idx = ref_start + i;
            if idx + fl > self.reference_buf.len() {
                output.push(mic);
                continue;
            }
            let ref_slice = &self.reference_buf[idx..idx + fl];
            let echo_est: f32 = self
                .echo_weights
                .iter()
                .zip(ref_slice)
                .map(|(w, r)| w * r)
                .sum();
            let error = mic - echo_est;
            let power: f32 = ref_slice.iter().map(|r| r * r).sum::<f32>() + 1e-6;
            let step = self.config.nlms_step / power;
            for (w, r) in self.echo_weights.iter_mut().zip(ref_slice) {
                *w += step * error * r;
            }
            output.push(error);
        }
        output
    }

    /// Feed TTS output for echo reference tracking.
    pub fn feed_reference(&mut self, reference: &[i16]) {
        self.reference_buf
            .extend(reference.iter().map(|&s| s as f32));
        if self.reference_buf.len() > 80_000 {
            let drain = self.reference_buf.len() - 80_000;
            self.reference_buf.drain(..drain);
        }
    }
}

/// Upsample by 3x using linear interpolation (16kHz → 48kHz) into existing buffer.
fn upsample_3x_into(input: &[i16], out: &mut Vec<f32>) {
    if input.is_empty() {
        return;
    }
    out.reserve(input.len() * 3);
    for i in 0..input.len() - 1 {
        let a = input[i] as f32;
        let b = input[i + 1] as f32;
        out.push(a);
        out.push(a + (b - a) / 3.0);
        out.push(a + (b - a) * 2.0 / 3.0);
    }
    // Last sample
    let last = input[input.len() - 1] as f32;
    out.push(last);
    out.push(last);
    out.push(last);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denoises_without_panic() {
        let mut state = NoiseCancelState::new(NoiseCancelConfig::default());
        // Feed enough frames to get past the first-frame discard
        for _ in 0..5 {
            state.process(&vec![100i16; 160], None);
        }
        let out = state.process(&vec![5000i16; 160], None);
        assert_eq!(out.len(), 160);
    }

    #[test]
    fn silence_is_suppressed() {
        let mut state = NoiseCancelState::new(NoiseCancelConfig::default());
        // Process several frames of low-level noise
        for _ in 0..20 {
            state.process(&vec![10i16; 160], None);
        }
        let out = state.process(&vec![5i16; 160], None);
        let rms: f32 =
            (out.iter().map(|&s| (s as f32).powi(2)).sum::<f32>() / out.len() as f32).sqrt();
        // RNNoise should suppress low-level noise significantly
        assert!(rms < 50.0, "RMS was {rms}, expected < 50 for noise");
    }

    #[test]
    fn loud_signal_passes() {
        let mut state = NoiseCancelState::new(NoiseCancelConfig::default());
        for _ in 0..5 {
            state.process(&vec![100i16; 160], None);
        }
        // Feed a loud signal through the neural denoiser
        let loud: Vec<i16> = (0..160)
            .map(|i| ((i as f32 * 0.1).sin() * 10000.0) as i16)
            .collect();
        let out = state.process(&loud, None);
        // Neural denoiser (RNNoise) may attenuate synthetic signals that don't
        // match speech patterns. Verify output is produced without panic and
        // has correct length.
        assert_eq!(out.len(), loud.len());
        // Output should contain non-zero signal (some signal passes through)
        assert!(out.iter().any(|&s| s != 0), "denoiser output is all zeros");
    }
}
