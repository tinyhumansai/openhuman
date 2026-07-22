//! Inference-side voice: local/cloud transcription (STT) and local TTS.
//!
//! Audio I/O, hotkeys, dictation, and the voice RPC surface remain in
//! `crate::openhuman::voice`. The files here are the actual inference
//! implementations that `voice/` imports.

pub mod cloud_transcribe;
pub mod hallucination;
pub mod local_speech;
pub mod local_transcribe;
pub mod postprocess;
// The dictation WebSocket handler (`handle_dictation_ws`) is the module's whole
// public surface and axum-only, and its sole caller is the gated core HTTP
// router (`core::jsonrpc::dictation_ws_handler`). The module is therefore
// exclusive to the `http-server` feature (#5048) — nothing else references it.
#[cfg(feature = "http-server")]
pub mod streaming;
