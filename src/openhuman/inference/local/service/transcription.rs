//! Provider-neutral transcription result type.
//!
//! Inert and dependency-free: it carries whatever confidence metadata an STT
//! engine can report, without naming one. It outlived the bundled whisper.cpp
//! engine that first introduced it — that engine is gone (STT is now
//! cloud/engine-configurable via `voice_server.stt_engine`), but the shape is
//! still the contract between an STT backend and the channel host adapter
//! (`channels::host::adapters`), so it stays here rather than being folded
//! into any one provider.

/// Result of a transcription call, including confidence metadata.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// The transcribed text (may be empty if all segments were rejected).
    pub text: String,
    /// Average log-probability across accepted segments (higher = more confident).
    /// `None` when the engine reports no per-segment confidence.
    pub avg_logprob: Option<f32>,
    /// Number of segments accepted / total segments produced by the engine.
    pub segments_accepted: usize,
    pub segments_total: usize,
}
