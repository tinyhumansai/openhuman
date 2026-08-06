//! Inert transcription result type — dependency-free and compiled in both
//! build states. The `inference` feature gates only the engine (`real`), not
//! this data type, so always-compiled callers (`../speech.rs`,
//! `inference::voice::streaming`) name one stable definition regardless of the
//! feature. See the module docs in `mod.rs` for the carve-out rationale.

/// Result of a transcription call, including confidence metadata.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// The transcribed text (may be empty if all segments were rejected).
    pub text: String,
    /// Average log-probability across accepted segments (higher = more confident).
    /// `None` if no segments were accepted.
    pub avg_logprob: Option<f32>,
    /// Number of segments accepted / total segments produced by Whisper.
    pub segments_accepted: usize,
    pub segments_total: usize,
}
