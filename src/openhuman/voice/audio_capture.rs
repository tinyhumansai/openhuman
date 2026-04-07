//! Microphone audio capture using cpal.
//!
//! Records audio from the default input device and produces 16-kHz mono WAV
//! bytes suitable for whisper transcription.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use hound::{SampleFormat as HoundFormat, WavSpec, WavWriter};
use log::{debug, error, info, warn};
use tokio::sync::oneshot;

const LOG_PREFIX: &str = "[voice_capture]";

/// Target sample rate for whisper (16 kHz mono).
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Result of a completed recording.
#[derive(Debug, Clone)]
pub struct RecordingResult {
    /// WAV-encoded audio bytes (16 kHz, mono, 16-bit PCM).
    pub wav_bytes: Vec<u8>,
    /// Duration of the recording in seconds.
    pub duration_secs: f32,
    /// Number of samples captured.
    pub sample_count: usize,
    /// Peak RMS energy observed during recording.
    /// Used for silence detection — values below ~0.002 indicate no speech.
    pub peak_rms: f32,
}

/// Handle to a recording in progress. Drop or call `stop()` to end recording.
pub struct RecordingHandle {
    stop_flag: Arc<AtomicBool>,
    result_rx: Option<oneshot::Receiver<Result<RecordingResult, String>>>,
}

impl RecordingHandle {
    /// Signal the recording to stop and return the captured audio.
    pub async fn stop(mut self) -> Result<RecordingResult, String> {
        self.stop_flag.store(true, Ordering::SeqCst);
        debug!("{LOG_PREFIX} stop signal sent");

        match self.result_rx.take() {
            Some(rx) => rx
                .await
                .map_err(|_| "recording task dropped before completing".to_string())?,
            None => Err("recording already stopped".to_string()),
        }
    }
}

/// Start recording from the default microphone.
///
/// Returns a `RecordingHandle` that must be `.stop().await`-ed to get
/// the captured audio. Recording runs on a dedicated OS thread because
/// `cpal::Stream` is `!Send` (it must be created and dropped on the
/// same thread).
pub fn start_recording() -> Result<RecordingHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let (result_tx, result_rx) = oneshot::channel();

    // Use a oneshot to report whether stream setup succeeded.
    let (setup_tx, setup_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);

    std::thread::Builder::new()
        .name("voice-capture".into())
        .spawn(move || {
            // All cpal objects are created and used on this thread.
            let result = record_on_thread(stop_flag_clone, setup_tx);
            let _ = result_tx.send(result);
        })
        .map_err(|e| format!("failed to spawn capture thread: {e}"))?;

    // Wait for the stream to be set up (or an error).
    match setup_rx.recv() {
        Ok(Ok(())) => {
            info!("{LOG_PREFIX} recording started");
            Ok(RecordingHandle {
                stop_flag,
                result_rx: Some(result_rx),
            })
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("capture thread exited before signalling readiness".to_string()),
    }
}

/// Runs the entire recording lifecycle on a single thread (cpal requirement).
fn record_on_thread(
    stop_flag: Arc<AtomicBool>,
    setup_tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<RecordingResult, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default audio input device found".to_string())?;

    let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
    info!("{LOG_PREFIX} using input device: {device_name}");

    let supported_configs = device
        .supported_input_configs()
        .map_err(|e| format!("failed to query input configs: {e}"))?;

    let config = find_best_config(supported_configs)?;
    let source_sample_rate = config.sample_rate().0;
    let source_channels = config.channels() as usize;

    debug!(
        "{LOG_PREFIX} recording config: rate={source_sample_rate} channels={source_channels} format={:?}",
        config.sample_format()
    );

    let samples: Arc<parking_lot::Mutex<Vec<f32>>> = Arc::new(parking_lot::Mutex::new(
        Vec::with_capacity(TARGET_SAMPLE_RATE as usize * 30),
    ));

    // Track peak RMS energy across the recording for silence detection.
    let peak_rms: Arc<std::sync::atomic::AtomicU32> =
        Arc::new(std::sync::atomic::AtomicU32::new(0));

    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.into();

    let stream = {
        let samples_writer = samples.clone();
        let rms_tracker = peak_rms.clone();
        match sample_format {
            SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mono = to_mono(data, source_channels);
                        update_peak_rms(&rms_tracker, &mono);
                        samples_writer.lock().extend_from_slice(&mono);
                    },
                    |err| error!("{LOG_PREFIX} audio stream error: {err}"),
                    None,
                )
                .map_err(|e| format!("failed to build f32 input stream: {e}")),
            SampleFormat::I16 => {
                let rms_tracker = peak_rms.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            let floats: Vec<f32> =
                                data.iter().map(|&s| s as f32 / 32768.0).collect();
                            let mono = to_mono(&floats, source_channels);
                            update_peak_rms(&rms_tracker, &mono);
                            samples_writer.lock().extend_from_slice(&mono);
                        },
                        |err| error!("{LOG_PREFIX} audio stream error: {err}"),
                        None,
                    )
                    .map_err(|e| format!("failed to build i16 input stream: {e}"))
            }
            SampleFormat::U16 => {
                let rms_tracker = peak_rms.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            let floats: Vec<f32> = data
                                .iter()
                                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                                .collect();
                            let mono = to_mono(&floats, source_channels);
                            update_peak_rms(&rms_tracker, &mono);
                            samples_writer.lock().extend_from_slice(&mono);
                        },
                        |err| error!("{LOG_PREFIX} audio stream error: {err}"),
                        None,
                    )
                    .map_err(|e| format!("failed to build u16 input stream: {e}"))
            }
            other => Err(format!("unsupported sample format: {other:?}")),
        }
    };

    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            let _ = setup_tx.send(Err(e.clone()));
            return Err(e);
        }
    };

    if let Err(e) = stream.play() {
        let msg = format!("failed to start audio stream: {e}");
        let _ = setup_tx.send(Err(msg.clone()));
        return Err(msg);
    }

    // Signal success so start_recording() returns.
    let _ = setup_tx.send(Ok(()));

    // Poll stop flag while keeping the stream alive on this thread.
    while !stop_flag.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    debug!("{LOG_PREFIX} stop flag detected, finalizing recording");
    drop(stream);

    let raw_samples = samples.lock().clone();
    let final_peak_rms = f32::from_bits(peak_rms.load(Ordering::Relaxed));
    debug!("{LOG_PREFIX} peak_rms={final_peak_rms:.6}");
    finalize_recording(raw_samples, source_sample_rate, final_peak_rms)
}

/// List available input devices.
pub fn list_input_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| format!("failed to enumerate input devices: {e}"))?;

    let names: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();

    debug!("{LOG_PREFIX} found {} input devices", names.len());
    Ok(names)
}

/// Convert interleaved multi-channel samples to mono by averaging channels.
fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }

    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resample mono f32 samples from `source_rate` to `TARGET_SAMPLE_RATE` using
/// linear interpolation. Good enough for voice dictation quality.
fn resample(samples: &[f32], source_rate: u32) -> Vec<f32> {
    if source_rate == TARGET_SAMPLE_RATE {
        return samples.to_vec();
    }

    let ratio = source_rate as f64 / TARGET_SAMPLE_RATE as f64;
    let output_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx0 = src_idx.floor() as usize;
        let idx1 = (idx0 + 1).min(samples.len().saturating_sub(1));
        let frac = (src_idx - idx0 as f64) as f32;
        output.push(samples[idx0] * (1.0 - frac) + samples[idx1] * frac);
    }

    output
}

/// Compute RMS energy for a chunk of mono samples and update the peak tracker.
/// Uses `AtomicU32` with `f32::to_bits`/`from_bits` for lock-free max tracking.
fn update_peak_rms(peak: &std::sync::atomic::AtomicU32, mono_samples: &[f32]) {
    if mono_samples.is_empty() {
        return;
    }
    let sum_sq: f32 = mono_samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / mono_samples.len() as f32).sqrt();
    // Atomic max via compare-and-swap loop.
    loop {
        let current_bits = peak.load(Ordering::Relaxed);
        let current = f32::from_bits(current_bits);
        if rms <= current {
            break;
        }
        if peak
            .compare_exchange_weak(
                current_bits,
                rms.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            break;
        }
    }
}

/// Finalize recorded samples into a 16-kHz mono WAV.
fn finalize_recording(
    raw_samples: Vec<f32>,
    source_sample_rate: u32,
    peak_rms: f32,
) -> Result<RecordingResult, String> {
    if raw_samples.is_empty() {
        warn!("{LOG_PREFIX} no audio samples captured");
        return Err("no audio samples captured".to_string());
    }

    let resampled = resample(&raw_samples, source_sample_rate);
    let sample_count = resampled.len();
    let duration_secs = sample_count as f32 / TARGET_SAMPLE_RATE as f32;

    debug!(
        "{LOG_PREFIX} finalizing: {sample_count} samples, {duration_secs:.1}s, \
         resampled from {source_sample_rate} to {TARGET_SAMPLE_RATE}"
    );

    let spec = WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: HoundFormat::Int,
    };

    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer =
            WavWriter::new(&mut buf, spec).map_err(|e| format!("WAV writer error: {e}"))?;

        for &sample in &resampled {
            let clamped = sample.clamp(-1.0, 1.0);
            let i16_sample = (clamped * 32767.0) as i16;
            writer
                .write_sample(i16_sample)
                .map_err(|e| format!("WAV write error: {e}"))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("WAV finalize error: {e}"))?;
    }

    let wav_bytes = buf.into_inner();
    info!(
        "{LOG_PREFIX} recording finalized: {duration_secs:.1}s, {} bytes WAV",
        wav_bytes.len()
    );

    Ok(RecordingResult {
        wav_bytes,
        duration_secs,
        sample_count,
        peak_rms,
    })
}

/// Find the best input config — prefer 16 kHz mono, else closest match.
fn find_best_config(
    configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> Result<cpal::SupportedStreamConfig, String> {
    let mut configs_vec: Vec<cpal::SupportedStreamConfigRange> = configs.collect();
    if configs_vec.is_empty() {
        return Err("no supported audio input configurations found".to_string());
    }

    // Sort: prefer configs whose range includes 16kHz, then by fewer channels.
    configs_vec.sort_by(|a, b| {
        let a_has_target = a.min_sample_rate().0 <= TARGET_SAMPLE_RATE
            && a.max_sample_rate().0 >= TARGET_SAMPLE_RATE;
        let b_has_target = b.min_sample_rate().0 <= TARGET_SAMPLE_RATE
            && b.max_sample_rate().0 >= TARGET_SAMPLE_RATE;

        b_has_target
            .cmp(&a_has_target)
            .then(a.channels().cmp(&b.channels()))
    });

    let best = &configs_vec[0];
    let rate = if best.min_sample_rate().0 <= TARGET_SAMPLE_RATE
        && best.max_sample_rate().0 >= TARGET_SAMPLE_RATE
    {
        SampleRate(TARGET_SAMPLE_RATE)
    } else {
        // Use the maximum supported rate and resample later.
        best.max_sample_rate()
    };

    Ok(best.clone().with_sample_rate(rate))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
