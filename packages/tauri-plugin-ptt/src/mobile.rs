/// iOS mobile bridge for tauri-plugin-ptt.
///
/// Tauri's `ios_plugin_binding!` macro generates the Swift<->Rust FFI glue.
/// Each command delegates to `PluginHandle::run_mobile_plugin`, which
/// serialises the payload to JSON, calls the matching Swift `@objc func` on
/// `PTTPlugin`, and deserialises the return value.
use serde::de::DeserializeOwned;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::{
    error::Result,
    models::{SpeakRequest, TranscriptResult, VoiceInfo},
};

// Generates `init_plugin_ptt` — the Swift entry-point symbol that
// `api.register_ios_plugin(init_plugin_ptt)` will call at startup.
tauri::ios_plugin_binding!(init_plugin_ptt);

pub struct PttMobile<R: Runtime>(PluginHandle<R>);

/// Construct and register the mobile plugin handle. Called from `lib.rs::init`.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<PttMobile<R>> {
    log::debug!("[ptt] mobile::init — registering iOS plugin handle");
    let handle = api.register_ios_plugin(init_plugin_ptt)?;
    Ok(PttMobile(handle))
}

impl<R: Runtime> PttMobile<R> {
    /// Begin a speech recognition session. Returns immediately; partial
    /// transcripts arrive as `ptt://transcript-partial` events.
    pub fn start_listening(&self) -> Result<()> {
        log::debug!("[ptt] mobile::start_listening");
        self.0
            .run_mobile_plugin::<()>("startListening", ())
            .map_err(Into::into)
    }

    /// Stop the active session and return the final transcript.
    pub fn stop_listening(&self) -> Result<TranscriptResult> {
        log::debug!("[ptt] mobile::stop_listening");
        self.0
            .run_mobile_plugin::<TranscriptResult>("stopListening", ())
            .map_err(Into::into)
    }

    /// Enqueue a TTS utterance. Returns once synthesis has been submitted.
    pub fn speak(&self, req: SpeakRequest) -> Result<()> {
        log::debug!("[ptt] mobile::speak text_len={}", req.text.len());
        self.0
            .run_mobile_plugin::<()>("speak", req)
            .map_err(Into::into)
    }

    /// Immediately stop any active TTS utterance.
    pub fn cancel_speech(&self) -> Result<()> {
        log::debug!("[ptt] mobile::cancel_speech");
        self.0
            .run_mobile_plugin::<()>("cancelSpeech", ())
            .map_err(Into::into)
    }

    /// Return available on-device voices from `AVSpeechSynthesisVoice.speechVoices()`.
    pub fn list_voices(&self) -> Result<Vec<VoiceInfo>> {
        log::debug!("[ptt] mobile::list_voices");
        self.0
            .run_mobile_plugin::<Vec<VoiceInfo>>("listVoices", ())
            .map_err(Into::into)
    }
}
