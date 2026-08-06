use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
    Wav,
}

impl AudioFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioGenerateRequest {
    pub text: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub format: Option<AudioFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailPodcastRequest {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub audio_path: String,
    #[serde(default)]
    pub attachment_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioGeneratedArtifact {
    pub output_path: String,
    pub file_name: String,
    pub provider: String,
    pub voice: Option<String>,
    pub format: AudioFormat,
    pub audio_mime: String,
    pub bytes_written: usize,
    pub chars_synthesized: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioEmailDeliveryResult {
    pub to: String,
    pub subject: String,
    pub attachment_name: String,
    pub mode: String,
    #[serde(default)]
    pub capture_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioToolkitGenerateAndEmailResult {
    pub audio: AudioGeneratedArtifact,
    pub email: AudioEmailDeliveryResult,
}
