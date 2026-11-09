//! RPC handler implementations for the voice domain.
//!
//! Handlers are split into two sub-modules by concern:
//! - `transcribe_tts`: transcription, synthesis, and factory-dispatch handlers
//! - `provider_server`: provider settings, model listing, and testing

mod provider_server;
mod transcribe_tts;

pub(super) use provider_server::{
    handle_voice_list_models, handle_voice_set_providers, handle_voice_test_provider,
    handle_voice_update_provider_settings, voice_list_models_controller,
    voice_set_providers_controller, voice_test_provider_controller,
    voice_update_provider_settings_controller,
};
pub(super) use transcribe_tts::{
    handle_voice_reply_synthesize, handle_voice_status, handle_voice_stt_dispatch,
    handle_voice_transcribe, handle_voice_transcribe_bytes, handle_voice_tts,
    handle_voice_tts_dispatch, voice_reply_synthesize_controller, voice_status_controller,
    voice_stt_dispatch_controller, voice_transcribe_bytes_controller, voice_transcribe_controller,
    voice_tts_controller, voice_tts_dispatch_controller,
};
