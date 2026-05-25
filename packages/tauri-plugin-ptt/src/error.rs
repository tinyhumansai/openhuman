use serde::Serialize;
use thiserror::Error;

/// Plugin-level errors returned to the JS caller.
#[derive(Debug, Error)]
pub enum Error {
    #[error("PTT is not supported on this platform")]
    NotSupported,
    #[error("microphone permission denied")]
    MicrophonePermissionDenied,
    #[error("speech recognition permission denied")]
    SpeechPermissionDenied,
    #[error("recording is already active")]
    AlreadyRecording,
    #[error("no active recording session")]
    NotRecording,
    #[error("audio engine error: {0}")]
    AudioEngine(String),
    #[error("speech recognizer error: {0}")]
    SpeechRecognizer(String),
    #[error("TTS error: {0}")]
    Tts(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),
    /// Mobile plugin invoke error (iOS only).
    #[cfg(mobile)]
    #[error("mobile plugin error: {0}")]
    MobilePlugin(#[from] tauri::plugin::mobile::PluginInvokeError),
}

/// Serialize to a JSON string for the JS boundary.
impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
