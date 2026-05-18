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
pub mod streaming;
