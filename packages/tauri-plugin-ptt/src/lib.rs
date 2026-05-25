/// tauri-plugin-ptt — push-to-talk + TTS plugin for Tauri v2 (iOS target).
///
/// Exposes five commands under the `ptt` plugin namespace:
///   - `start_listening`  — activate AVAudioEngine + SFSpeechRecognizer
///   - `stop_listening`   — deactivate and return final transcript
///   - `speak`            — enqueue an AVSpeechSynthesizer utterance
///   - `cancel_speech`    — stop current utterance immediately
///   - `list_voices`      — enumerate on-device TTS voices
///
/// Events emitted (Tauri event bus, target "main"):
///   - `ptt://transcript-partial`  { text }
///   - `ptt://transcript-final`    { text }
///   - `ptt://tts-started`         { utteranceId }
///   - `ptt://tts-ended`           { utteranceId, finished }
///   - `ptt://error`               { code, message }
///
/// Desktop: all commands return `Error::NotSupported`.
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

mod commands;
mod error;
mod models;

#[cfg(target_os = "ios")]
mod mobile;

pub use error::{Error, Result};
pub use models::*;

// ── PttHandle — cross-platform façade ────────────────────────────────────────

/// State token managed by Tauri. On iOS it wraps the `PluginHandle`;
/// on desktop it is a zero-cost stub that returns `NotSupported`.
///
/// `fn(R) -> R` phantom is used instead of `PhantomData<R>` so the struct
/// is always `Send + Sync` regardless of whether `R` is `Send + Sync`
/// (Tauri's `manage()` requires `Send + Sync + 'static`).
pub struct PttHandle<R: Runtime> {
    #[cfg(target_os = "ios")]
    inner_mobile: mobile::PttMobile<R>,
    #[cfg(not(target_os = "ios"))]
    _marker: std::marker::PhantomData<fn(R) -> R>,
}

// SAFETY: PttHandle contains only a PluginHandle<R> (mobile) or PhantomData
// (desktop). PluginHandle is Send + Sync, and fn(R)->R phantom is Send + Sync.
unsafe impl<R: Runtime> Send for PttHandle<R> {}
unsafe impl<R: Runtime> Sync for PttHandle<R> {}

impl<R: Runtime> PttHandle<R> {
    #[cfg(target_os = "ios")]
    fn new(inner: mobile::PttMobile<R>) -> Self {
        Self {
            inner_mobile: inner,
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn new_stub() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    pub fn start_listening(&self) -> Result<()> {
        #[cfg(target_os = "ios")]
        return self.inner_mobile.start_listening();
        #[cfg(not(target_os = "ios"))]
        {
            log::warn!("[ptt] start_listening called on non-mobile target — not supported");
            Err(Error::NotSupported)
        }
    }

    pub fn stop_listening(&self) -> Result<crate::models::TranscriptResult> {
        #[cfg(target_os = "ios")]
        return self.inner_mobile.stop_listening();
        #[cfg(not(target_os = "ios"))]
        {
            log::warn!("[ptt] stop_listening called on non-mobile target — not supported");
            Err(Error::NotSupported)
        }
    }

    pub fn speak(&self, req: crate::models::SpeakRequest) -> Result<()> {
        #[cfg(target_os = "ios")]
        return self.inner_mobile.speak(req);
        #[cfg(not(target_os = "ios"))]
        {
            let _ = req;
            log::warn!("[ptt] speak called on non-mobile target — not supported");
            Err(Error::NotSupported)
        }
    }

    pub fn cancel_speech(&self) -> Result<()> {
        #[cfg(target_os = "ios")]
        return self.inner_mobile.cancel_speech();
        #[cfg(not(target_os = "ios"))]
        {
            log::warn!("[ptt] cancel_speech called on non-mobile target — not supported");
            Err(Error::NotSupported)
        }
    }

    pub fn list_voices(&self) -> Result<Vec<crate::models::VoiceInfo>> {
        #[cfg(target_os = "ios")]
        return self.inner_mobile.list_voices();
        #[cfg(not(target_os = "ios"))]
        {
            log::warn!("[ptt] list_voices called on non-mobile target — not supported");
            Err(Error::NotSupported)
        }
    }
}

// ── Plugin init ──────────────────────────────────────────────────────────────

/// Initialise the PTT plugin and return a `TauriPlugin` for registration.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    log::debug!("[ptt] init — building plugin");

    Builder::new("ptt")
        .invoke_handler(tauri::generate_handler![
            commands::start_listening,
            commands::stop_listening,
            commands::speak,
            commands::cancel_speech,
            commands::list_voices,
        ])
        .setup(|app, api| {
            log::debug!("[ptt] setup — configuring platform bridge");

            #[cfg(target_os = "ios")]
            {
                let mobile_handle = mobile::init(app, api)?;
                let handle = PttHandle::new(mobile_handle);
                app.manage(handle);
                log::info!("[ptt] iOS bridge registered");
            }
            #[cfg(not(target_os = "ios"))]
            {
                let _ = (app, api);
                let handle: PttHandle<R> = PttHandle::new_stub();
                app.manage(handle);
                log::debug!("[ptt] non-mobile target — plugin registered as no-op stub");
            }

            Ok(())
        })
        .build()
}
