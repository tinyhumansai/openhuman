//! Disabled facade for the whisper engine — compiled when the `inference`
//! feature is OFF (`whisper-rs` and `cpal` are dropped from the build).
//!
//! Mirrors `real`'s public function + handle surface exactly so the
//! always-compiled callers (`../speech.rs`, `../bootstrap.rs`,
//! `inference::voice::streaming`, and the voice STT factory when `voice` is on)
//! need no per-call `#[cfg]`. Every transcription path returns the disabled
//! error; loading is a no-op and nothing is ever "loaded".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use super::types::TranscriptionResult;

const DISABLED: &str = "in-process whisper STT is disabled: this build was compiled without the `inference` feature (rebuild with `--features inference`)";

/// Whisper-free mirror of the real engine handle. Always empty — with the
/// engine compiled out nothing loads — but keeps the same
/// `Arc<Mutex<Option<_>>>` shape so `LocalAiService.whisper` still constructs.
pub type WhisperEngineHandle = Arc<Mutex<Option<()>>>;

/// Create a new (permanently empty) engine handle.
pub fn new_handle() -> WhisperEngineHandle {
    Arc::new(Mutex::new(None))
}

/// No-op: there is no engine to load. Returns the disabled error so callers
/// log/fall back exactly as they would on a real load failure.
pub fn load_engine(
    _handle: &WhisperEngineHandle,
    _model_path: &Path,
    _has_gpu: bool,
    _gpu_description: Option<&str>,
) -> Result<(), String> {
    log::debug!("[whisper_engine::stub] load_engine no-op — {DISABLED}");
    Err(DISABLED.to_string())
}

/// No-op: nothing is ever loaded.
pub fn unload_engine(_handle: &WhisperEngineHandle) {}

/// Always `false` — the in-process engine is compiled out.
pub fn is_loaded(_handle: &WhisperEngineHandle) -> bool {
    false
}

/// Always `None` — no model can be loaded.
pub fn loaded_model_path(_handle: &WhisperEngineHandle) -> Option<PathBuf> {
    None
}

pub fn transcribe_pcm_f32(
    _handle: &WhisperEngineHandle,
    _audio_f32: &[f32],
    _language: Option<&str>,
    _initial_prompt: Option<&str>,
) -> Result<TranscriptionResult, String> {
    log::debug!("[whisper] transcribe_pcm_f32 unavailable: built without the `inference` feature");
    Err(DISABLED.to_string())
}

pub fn transcribe_pcm_i16(
    _handle: &WhisperEngineHandle,
    _audio_i16: &[i16],
    _language: Option<&str>,
    _initial_prompt: Option<&str>,
) -> Result<TranscriptionResult, String> {
    log::debug!("[whisper] transcribe_pcm_i16 unavailable: built without the `inference` feature");
    Err(DISABLED.to_string())
}

pub fn transcribe_wav_file(
    _handle: &WhisperEngineHandle,
    _wav_path: &Path,
    _language: Option<&str>,
    _initial_prompt: Option<&str>,
) -> Result<TranscriptionResult, String> {
    log::debug!("[whisper] transcribe_wav_file unavailable: built without the `inference` feature");
    Err(DISABLED.to_string())
}

/// Cheap RIFF/WAVE header sniff — dependency-free, so the stub keeps the real
/// behaviour rather than a misleading constant (matches `real::looks_like_wav`).
pub(crate) fn looks_like_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

pub(crate) fn transcribe_wav_bytes(
    _handle: &WhisperEngineHandle,
    _wav_bytes: &[u8],
    _language: Option<&str>,
    _initial_prompt: Option<&str>,
) -> Result<TranscriptionResult, String> {
    log::debug!(
        "[whisper] transcribe_wav_bytes unavailable: built without the `inference` feature"
    );
    Err(DISABLED.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_handle_never_loads_and_transcribe_errors() {
        let h = new_handle();
        assert!(!is_loaded(&h));
        assert!(loaded_model_path(&h).is_none());
        let err = transcribe_pcm_f32(&h, &[0.0; 16], None, None).unwrap_err();
        assert!(
            err.contains("inference"),
            "disabled error names the gate: {err}"
        );
        // i16 + wav paths error the same way.
        assert!(transcribe_pcm_i16(&h, &[0i16; 16], None, None).is_err());
        assert!(transcribe_wav_bytes(&h, b"not a wav", None, None).is_err());
    }

    #[test]
    fn stub_looks_like_wav_matches_real_behaviour() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&[0u8; 4]);
        wav.extend_from_slice(b"WAVE");
        assert!(looks_like_wav(&wav));
        assert!(!looks_like_wav(b"OggS...."));
        assert!(!looks_like_wav(b"RIFF"));
    }
}
