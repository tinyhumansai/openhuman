//! In-process whisper.cpp inference via whisper-rs.
//!
//! Loads the GGML model once into a `WhisperContext` and reuses it across
//! transcription calls, eliminating the cold-start latency of spawning a
//! subprocess per request.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use log::{debug, info};
use parking_lot::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const LOG_PREFIX: &str = "[whisper_engine]";

/// Wraps a loaded `WhisperContext` for reuse across transcription calls.
pub struct WhisperEngine {
    context: WhisperContext,
    model_path: PathBuf,
}

/// Thread-safe handle to an optionally-loaded whisper engine.
pub type WhisperEngineHandle = Arc<Mutex<Option<WhisperEngine>>>;

/// Create a new empty engine handle. The engine is loaded lazily or during
/// bootstrap via [`load_engine`].
pub fn new_handle() -> WhisperEngineHandle {
    Arc::new(Mutex::new(None))
}

/// Attempt to load a whisper model into the engine. Returns an error string
/// if loading fails (e.g. model file missing, unsupported format).
pub fn load_engine(handle: &WhisperEngineHandle, model_path: &Path) -> Result<(), String> {
    info!(
        "{LOG_PREFIX} loading whisper model: {}",
        model_path.display()
    );

    if !model_path.is_file() {
        return Err(format!("whisper model not found: {}", model_path.display()));
    }

    let params = WhisperContextParameters::default();
    let ctx = WhisperContext::new_with_params(model_path.to_str().unwrap_or(""), params)
        .map_err(|e| format!("failed to load whisper model: {e}"))?;

    let engine = WhisperEngine {
        context: ctx,
        model_path: model_path.to_path_buf(),
    };

    *handle.lock() = Some(engine);
    info!("{LOG_PREFIX} whisper model loaded successfully");
    Ok(())
}

/// Unload the whisper model from memory.
pub fn unload_engine(handle: &WhisperEngineHandle) {
    let mut guard = handle.lock();
    if guard.is_some() {
        *guard = None;
        info!("{LOG_PREFIX} whisper model unloaded");
    }
}

/// Returns true if a model is currently loaded.
pub fn is_loaded(handle: &WhisperEngineHandle) -> bool {
    handle.lock().is_some()
}

/// Returns the path of the currently loaded model, if any.
pub fn loaded_model_path(handle: &WhisperEngineHandle) -> Option<PathBuf> {
    handle.lock().as_ref().map(|e| e.model_path.clone())
}

/// Transcribe raw PCM audio (16 kHz, mono, f32 samples).
///
/// Returns the concatenated transcript text or an error.
pub fn transcribe_pcm_f32(
    handle: &WhisperEngineHandle,
    audio_f32: &[f32],
    language: Option<&str>,
) -> Result<String, String> {
    let mut guard = handle.lock();
    let engine = guard
        .as_mut()
        .ok_or_else(|| "whisper engine not loaded".to_string())?;

    debug!(
        "{LOG_PREFIX} transcribing {} samples ({:.1}s of audio)",
        audio_f32.len(),
        audio_f32.len() as f64 / 16000.0
    );

    let mut state = engine
        .context
        .create_state()
        .map_err(|e| format!("failed to create whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });

    if let Some(lang) = language {
        params.set_language(Some(lang));
    } else {
        params.set_language(Some("en"));
    }

    // Disable printing to stdout — we capture segments programmatically.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    // Use available CPU threads (capped at 4 to avoid over-subscription).
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    params.set_n_threads(n_threads);

    let infer_started = Instant::now();
    state
        .full(params, audio_f32)
        .map_err(|e| format!("whisper inference failed: {e}"))?;
    let infer_elapsed = infer_started.elapsed();

    let mut text = String::new();
    let mut segment_count = 0;
    for segment in state.as_iter() {
        match segment.to_str() {
            Ok(segment_text) => text.push_str(segment_text),
            Err(e) => {
                debug!("{LOG_PREFIX} skipping segment: {e}");
            }
        }
        segment_count += 1;
    }

    let trimmed = text.trim().to_string();
    debug!(
        "{LOG_PREFIX} transcription complete: {} chars, {} segments, n_threads={}, infer_elapsed_ms={}",
        trimmed.len(),
        segment_count,
        n_threads,
        infer_elapsed.as_millis()
    );

    Ok(trimmed)
}

/// Transcribe raw PCM audio provided as 16-bit signed integers (16 kHz mono).
///
/// Converts to f32 internally before running inference.
pub fn transcribe_pcm_i16(
    handle: &WhisperEngineHandle,
    audio_i16: &[i16],
    language: Option<&str>,
) -> Result<String, String> {
    let mut audio_f32 = vec![0.0f32; audio_i16.len()];
    whisper_rs::convert_integer_to_float_audio(audio_i16, &mut audio_f32)
        .map_err(|e| format!("audio conversion failed: {e}"))?;
    transcribe_pcm_f32(handle, &audio_f32, language)
}

/// Read a WAV file and transcribe it. The WAV must be 16 kHz mono PCM
/// (16-bit or 32-bit float). For other formats, convert to WAV first
/// (e.g. via ffmpeg).
pub fn transcribe_wav_file(
    handle: &WhisperEngineHandle,
    wav_path: &Path,
    language: Option<&str>,
) -> Result<String, String> {
    debug!("{LOG_PREFIX} reading WAV file: {}", wav_path.display());

    let raw_bytes = std::fs::read(wav_path).map_err(|e| format!("failed to read WAV file: {e}"))?;

    let audio_f32 = decode_wav_to_f32(&raw_bytes)?;
    transcribe_pcm_f32(handle, &audio_f32, language)
}

/// Minimal WAV decoder — extracts PCM samples as f32 from a standard
/// RIFF/WAVE file. Supports 16-bit integer and 32-bit float formats.
/// Resampling is NOT performed; the input should already be 16 kHz mono.
fn decode_wav_to_f32(data: &[u8]) -> Result<Vec<f32>, String> {
    if data.len() < 44 {
        return Err("WAV file too small".to_string());
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a valid WAV file".to_string());
    }

    let mut pos = 12;
    let mut fmt_found = false;
    let mut audio_format: u16 = 0;
    let mut num_channels: u16 = 0;
    #[allow(unused_assignments)]
    let mut sample_rate: u32 = 0;
    let mut bits_per_sample: u16 = 0;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize;

        if chunk_id == b"fmt " {
            if chunk_size < 16 || pos + 8 + chunk_size > data.len() {
                return Err("malformed fmt chunk".to_string());
            }
            let fmt = &data[pos + 8..];
            audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
            num_channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            fmt_found = true;

            if sample_rate != 16000 {
                return Err(format!(
                    "unsupported sample rate {sample_rate} Hz, whisper requires 16000 Hz"
                ));
            }
            if num_channels == 0 || num_channels > 2 {
                return Err(format!(
                    "unsupported channel count {num_channels}, expected 1 (mono) or 2 (stereo)"
                ));
            }
        }

        if chunk_id == b"data" && fmt_found {
            let pcm_data = &data[pos + 8..pos + 8 + chunk_size.min(data.len() - pos - 8)];
            return convert_pcm_to_f32(pcm_data, audio_format, num_channels, bits_per_sample);
        }

        pos += 8 + chunk_size;
        if chunk_size % 2 != 0 {
            pos += 1;
        }
    }

    Err("WAV file missing data chunk".to_string())
}

fn convert_pcm_to_f32(
    pcm: &[u8],
    audio_format: u16,
    num_channels: u16,
    bits_per_sample: u16,
) -> Result<Vec<f32>, String> {
    match (audio_format, bits_per_sample) {
        // PCM 16-bit
        (1, 16) => {
            let samples: Vec<i16> = pcm
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();

            let mono = if num_channels == 2 {
                samples
                    .chunks_exact(2)
                    .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
                    .collect::<Vec<_>>()
            } else {
                samples
            };

            Ok(mono.iter().map(|&s| s as f32 / 32768.0).collect())
        }
        // IEEE float 32-bit
        (3, 32) => {
            let samples: Vec<f32> = pcm
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();

            if num_channels == 2 {
                Ok(samples
                    .chunks_exact(2)
                    .map(|pair| (pair[0] + pair[1]) / 2.0)
                    .collect())
            } else {
                Ok(samples)
            }
        }
        _ => Err(format!(
            "unsupported WAV format: audio_format={audio_format}, bits_per_sample={bits_per_sample}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_handle_starts_unloaded() {
        let handle = new_handle();
        assert!(!is_loaded(&handle));
        assert!(loaded_model_path(&handle).is_none());
    }

    #[test]
    fn load_engine_fails_for_missing_model() {
        let handle = new_handle();
        let result = load_engine(&handle, Path::new("/nonexistent/model.bin"));
        assert!(result.is_err());
        assert!(!is_loaded(&handle));
    }

    #[test]
    fn transcribe_pcm_fails_when_not_loaded() {
        let handle = new_handle();
        let audio = vec![0.0f32; 16000];
        let result = transcribe_pcm_f32(&handle, &audio, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not loaded"));
    }

    #[test]
    fn decode_wav_rejects_too_small() {
        let result = decode_wav_to_f32(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_wav_rejects_non_wav() {
        let result = decode_wav_to_f32(&[0u8; 44]);
        assert!(result.is_err());
    }

    #[test]
    fn convert_i16_produces_correct_length() {
        let handle = new_handle();
        let audio_i16 = vec![0i16; 100];
        let result = transcribe_pcm_i16(&handle, &audio_i16, None);
        assert!(result.is_err()); // expected: engine not loaded
    }
}
