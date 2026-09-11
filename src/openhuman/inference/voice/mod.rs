//! Inference-side voice: hosted transcription (STT) and local Piper TTS.
//!
//! Audio I/O, hotkeys, dictation, and the voice RPC surface remain in
//! `crate::openhuman::voice`. The files here are the actual inference
//! implementations that `voice/` imports.

pub mod cloud_transcribe;
pub mod local_speech;
pub mod postprocess;
// The dictation WebSocket handler (`handle_dictation_ws`) is the module's whole
// public surface and axum-only, and its sole caller is the gated core HTTP
// router (`core::jsonrpc::dictation_ws_handler`), so it needs `http-server`
// (#5048).
//
// It needs `voice` as well, which used to be true only by accident. The router
// reaches this through the `voice::streaming` re-export, and that re-export is
// already `all(voice, http-server)` — with `voice` off the router gets
// `voice::stub::streaming` instead, and this module compiled without a single
// caller. Making the gate say so removes that dead compilation, and is load
// bearing now that the WAV framing here calls the `tinyvoice` module: `modules`
// arrives via `voice`, so an `http-server`-only build would reference a
// namespace that is not there.
#[cfg(all(feature = "voice", feature = "http-server"))]
pub mod streaming;
