/// Tauri commands exposed to the JS layer via `plugin:ptt|<name>`.
///
/// `_app: AppHandle<R>` is included in each command so `generate_handler!` can
/// infer the runtime type parameter `R`. This matches the pattern used by other
/// Tauri v2 plugins (e.g. tauri-plugin-notification).
use tauri::{command, AppHandle, Runtime, State};

use crate::{
    error::Result,
    models::{SpeakRequest, TranscriptResult, VoiceInfo},
    PttHandle,
};

// ── start_listening ──────────────────────────────────────────────────────────

/// Begin a push-to-talk recording session.
///
/// Activates the `AVAudioEngine` and `SFSpeechRecognizer` pipeline on iOS.
/// Partial transcripts arrive as `ptt://transcript-partial` Tauri events.
#[command]
pub async fn start_listening<R: Runtime>(
    _app: AppHandle<R>,
    ptt: State<'_, PttHandle<R>>,
) -> Result<()> {
    log::debug!("[ptt] command: start_listening");
    ptt.inner().start_listening()
}

// ── stop_listening ───────────────────────────────────────────────────────────

/// Stop the active recording session.
///
/// Returns the final recognized text. Also emits `ptt://transcript-final`.
#[command]
pub async fn stop_listening<R: Runtime>(
    _app: AppHandle<R>,
    ptt: State<'_, PttHandle<R>>,
) -> Result<TranscriptResult> {
    log::debug!("[ptt] command: stop_listening");
    let result = ptt.inner().stop_listening()?;
    log::debug!(
        "[ptt] stop_listening returned text_len={}",
        result.text.len()
    );
    Ok(result)
}

// ── speak ────────────────────────────────────────────────────────────────────

/// Enqueue a TTS utterance via `AVSpeechSynthesizer`.
///
/// `voice_id` is an optional `AVSpeechSynthesisVoice.identifier`.
/// `rate` is a float in [0.5, 2.0] where 1.0 = normal speed.
#[command]
pub async fn speak<R: Runtime>(
    _app: AppHandle<R>,
    ptt: State<'_, PttHandle<R>>,
    text: String,
    voice_id: Option<String>,
    rate: Option<f32>,
) -> Result<()> {
    log::debug!("[ptt] command: speak text_len={}", text.len());
    ptt.inner().speak(SpeakRequest {
        text,
        voice_id,
        rate,
    })
}

// ── cancel_speech ────────────────────────────────────────────────────────────

/// Immediately stop any in-progress TTS utterance.
#[command]
pub async fn cancel_speech<R: Runtime>(
    _app: AppHandle<R>,
    ptt: State<'_, PttHandle<R>>,
) -> Result<()> {
    log::debug!("[ptt] command: cancel_speech");
    ptt.inner().cancel_speech()
}

// ── list_voices ──────────────────────────────────────────────────────────────

/// List all on-device TTS voices available via `AVSpeechSynthesisVoice.speechVoices()`.
#[command]
pub async fn list_voices<R: Runtime>(
    _app: AppHandle<R>,
    ptt: State<'_, PttHandle<R>>,
) -> Result<Vec<VoiceInfo>> {
    log::debug!("[ptt] command: list_voices");
    ptt.inner().list_voices()
}
