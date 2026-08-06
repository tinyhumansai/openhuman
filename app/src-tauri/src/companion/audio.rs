//! Native microphone capture for the desktop companion (shell-side).
//!
//! This is the "shell captures audio natively" half of the migration: while a
//! companion session is Listening we open the default input device with `cpal`,
//! downmix to mono `i16`, and resample to ~16 kHz PCM for upload to the core STT
//! endpoint (`openhuman.voice_transcribe_bytes`). We deliberately do **not**
//! depend on the core voice listener for capture.
//!
//! `cpal::Stream` is `!Send` on several platforms, so the stream is built and
//! owned by a dedicated std thread; the audio callback accumulates samples into
//! a shared buffer and the thread holds the stream alive until `stop()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{debug, info, warn};

const LOG_PREFIX: &str = "[companion_audio]";

/// Target sample rate for STT upload. Whisper works at 16 kHz mono.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// A live microphone capture. Dropping without `stop()` still tears the stream
/// down (via the `running` flag + thread join in `Drop`).
pub struct MicCapture {
    running: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<i16>>>,
    device_rate: u32,
    handle: Option<JoinHandle<()>>,
}

impl MicCapture {
    /// Open the default input device and begin capturing mono `i16` at the
    /// device's native rate. Returns once the stream is playing (or an error).
    pub fn start() -> Result<Self, String> {
        let running = Arc::new(AtomicBool::new(true));
        let buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));

        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let thread_running = running.clone();
        let thread_buffer = buffer.clone();

        let handle = std::thread::Builder::new()
            .name("companion-mic".into())
            .spawn(move || {
                run_capture_thread(thread_running, thread_buffer, ready_tx);
            })
            .map_err(|e| format!("failed to spawn mic thread: {e}"))?;

        let device_rate = match ready_rx.recv() {
            Ok(Ok(rate)) => rate,
            Ok(Err(e)) => {
                running.store(false, Ordering::SeqCst);
                let _ = handle.join();
                return Err(e);
            }
            Err(_) => {
                running.store(false, Ordering::SeqCst);
                let _ = handle.join();
                return Err("mic capture thread exited before signalling ready".into());
            }
        };

        info!("{LOG_PREFIX} capture started device_rate={device_rate}");
        Ok(Self {
            running,
            buffer,
            device_rate,
            handle: Some(handle),
        })
    }

    /// Stop capture and return the collected PCM resampled to
    /// [`TARGET_SAMPLE_RATE`] (mono `i16`).
    pub fn stop(mut self) -> (Vec<i16>, u32) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let raw = std::mem::take(&mut *self.buffer.lock().unwrap());
        debug!(
            "{LOG_PREFIX} capture stopped raw_samples={} device_rate={}",
            raw.len(),
            self.device_rate
        );
        let resampled = resample_linear(&raw, self.device_rate, TARGET_SAMPLE_RATE);
        (resampled, TARGET_SAMPLE_RATE)
    }
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Body of the capture thread: build the input stream, signal readiness with the
/// device sample rate, then keep the stream alive until told to stop.
fn run_capture_thread(
    running: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<i16>>>,
    ready_tx: mpsc::Sender<Result<u32, String>>,
) {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            let _ = ready_tx.send(Err("no default input device".into()));
            return;
        }
    };

    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("no default input config: {e}")));
            return;
        }
    };

    let device_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let err_fn = |e| warn!("{LOG_PREFIX} input stream error: {e}");

    let build = |buf: Arc<Mutex<Vec<i16>>>| -> Result<cpal::Stream, cpal::BuildStreamError> {
        match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let mut guard = buf.lock().unwrap();
                    for frame in data.chunks(channels.max(1)) {
                        let sum: f32 = frame.iter().copied().sum();
                        let mono = sum / channels.max(1) as f32;
                        guard.push((mono.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let mut guard = buf.lock().unwrap();
                    for frame in data.chunks(channels.max(1)) {
                        let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                        guard.push((sum / channels.max(1) as i32) as i16);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let mut guard = buf.lock().unwrap();
                    for frame in data.chunks(channels.max(1)) {
                        let sum: i32 = frame.iter().map(|&s| s as i32 - 32768).sum();
                        guard.push((sum / channels.max(1) as i32) as i16);
                    }
                },
                err_fn,
                None,
            ),
            other => Err(cpal::BuildStreamError::BackendSpecific {
                err: cpal::BackendSpecificError {
                    description: format!("unsupported sample format: {other:?}"),
                },
            }),
        }
    };

    let stream = match build(buffer) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("failed to build input stream: {e}")));
            return;
        }
    };

    if let Err(e) = stream.play() {
        let _ = ready_tx.send(Err(format!("failed to start input stream: {e}")));
        return;
    }

    let _ = ready_tx.send(Ok(device_rate));

    // Keep the stream alive on this thread until stop() flips the flag.
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
    debug!("{LOG_PREFIX} capture thread exiting");
}

/// Naive linear-interpolation resampler for mono `i16`. Adequate for speech STT
/// upload; whisper is robust to modest resampling artefacts.
fn resample_linear(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let idx = src.floor() as usize;
        let frac = src - idx as f64;
        let a = input[idx.min(input.len() - 1)] as f64;
        let b = input[(idx + 1).min(input.len() - 1)] as f64;
        out.push((a + (b - a) * frac).round() as i16);
    }
    out
}

/// Pack mono `i16` little-endian PCM into a minimal RIFF/WAVE container.
///
/// The core STT endpoint (`openhuman.voice_transcribe_bytes`) takes container
/// bytes + an `extension`, not raw PCM — and the shell must not call the
/// `meet`-gated `meet_agent::wav` packer, so we ship this tiny dependency-free
/// RIFF writer here.
pub fn pack_wav_pcm16le_mono(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len() as u32;
    let bytes_per_sample = 2u32;
    let byte_rate = sample_rate * bytes_per_sample; // mono
    let data_len = num_samples * bytes_per_sample;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels = mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&(bytes_per_sample as u16).to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
                                                 // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity_when_rates_match() {
        let input = vec![1i16, 2, 3, 4];
        assert_eq!(resample_linear(&input, 16000, 16000), input);
    }

    #[test]
    fn resample_downsamples_length() {
        let input = vec![0i16; 48_000];
        let out = resample_linear(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_empty_is_empty() {
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn wav_header_is_44_bytes_plus_data() {
        let samples = vec![0i16, 1, -1, 32767, -32768];
        let wav = pack_wav_pcm16le_mono(&samples, 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        // data length field matches
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_len as usize, samples.len() * 2);
    }

    #[test]
    fn wav_sample_rate_field_roundtrips() {
        let wav = pack_wav_pcm16le_mono(&[0i16; 4], 16_000);
        let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(rate, 16_000);
    }
}
