//! RPC handler implementations for the voice domain.
//!
//! Handlers are split into two sub-modules by concern:
//! - `transcribe_tts`: transcription, synthesis, and factory-dispatch handlers
//! - `provider_server`: provider settings, model listing, testing, and server lifecycle

mod provider_server;
mod transcribe_tts;

pub(super) use provider_server::{
    handle_overlay_stt_notify, handle_voice_list_models, handle_voice_server_start,
    handle_voice_server_status, handle_voice_server_stop, handle_voice_set_providers,
    handle_voice_test_provider, handle_voice_update_provider_settings,
};
pub(super) use transcribe_tts::{
    handle_voice_cloud_transcribe, handle_voice_reply_synthesize, handle_voice_status,
    handle_voice_stt_dispatch, handle_voice_transcribe, handle_voice_transcribe_bytes,
    handle_voice_tts, handle_voice_tts_dispatch,
};
