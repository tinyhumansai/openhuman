//! In-process whisper.cpp STT engine, gated behind the `inference` feature.
//!
//! This is the whisper/`cpal` dependency shed the `voice` gate deferred (see
//! the `inference` feature in the root `Cargo.toml`). Structure follows the
//! type-carve-out variant of the repo's facade + stub pattern (AGENTS.md):
//!
//! - [`types`] holds `TranscriptionResult` — an inert, dependency-free data
//!   type named by always-compiled callers (the local-AI service in
//!   `../speech.rs`/`../bootstrap.rs`) and by `inference::voice::streaming`.
//!   It stays compiled in **both** build states, so its fields can never drift.
//! - `real` owns the actual `whisper-rs` / `WhisperContext` engine and is
//!   compiled only with `--features inference`.
//! - `stub` mirrors `real`'s function + handle surface exactly with
//!   disabled-error / no-op bodies, so those always-compiled callers need no
//!   per-call `#[cfg]`.
//!
//! The stub signatures must match `real` exactly — the disabled build
//! (`--no-default-features --features tokenjuice-treesitter`) is the only thing
//! that catches drift, so run it after touching either side.

mod types;
// The facade re-exports the whole engine surface, but which items a given build
// actually names depends on downstream feature gates — the voice STT factory
// (`voice`) consumes `looks_like_wav` / `transcribe_wav_bytes` /
// `loaded_model_path` etc., while the always-compiled local-AI service uses only
// a subset. Allow unused re-exports so the enabled and disabled builds keep an
// identical public surface instead of drifting on which subset they pull.
#[allow(unused_imports)]
pub use types::TranscriptionResult;

#[cfg(feature = "inference")]
mod real;
#[cfg(feature = "inference")]
#[allow(unused_imports)]
pub use real::{
    is_loaded, load_engine, loaded_model_path, new_handle, transcribe_pcm_f32, transcribe_pcm_i16,
    transcribe_wav_file, unload_engine, WhisperEngineHandle,
};
#[cfg(feature = "inference")]
#[allow(unused_imports)]
pub(crate) use real::{looks_like_wav, transcribe_wav_bytes};

#[cfg(not(feature = "inference"))]
mod stub;
#[cfg(not(feature = "inference"))]
#[allow(unused_imports)]
pub use stub::{
    is_loaded, load_engine, loaded_model_path, new_handle, transcribe_pcm_f32, transcribe_pcm_i16,
    transcribe_wav_file, unload_engine, WhisperEngineHandle,
};
#[cfg(not(feature = "inference"))]
#[allow(unused_imports)]
pub(crate) use stub::{looks_like_wav, transcribe_wav_bytes};
