use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::audio_toolkit::{
    email_podcast, generate_and_email_podcast, generate_podcast, AudioGenerateRequest,
    EmailPodcastRequest,
};
use crate::openhuman::config::Config;
use crate::openhuman::security::{SecurityPolicy, ToolOperation};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

pub struct AudioGeneratePodcastTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

pub struct AudioEmailPodcastTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

pub struct AudioGenerateAndEmailPodcastTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl AudioGeneratePodcastTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

impl AudioEmailPodcastTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

impl AudioGenerateAndEmailPodcastTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

#[async_trait]
impl Tool for AudioGeneratePodcastTool {
    fn name(&self) -> &str {
        "audio_generate_podcast"
    }

    fn description(&self) -> &str {
        "Generate an audio file from text and save it into the workspace for listen-later or podcast-style delivery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "description": "Text to synthesize into audio." },
                "title": { "type": "string", "description": "Optional title used in the default file name." },
                "output_path": { "type": "string", "description": "Optional workspace-relative path to write the audio file to." },
                "provider": { "type": "string", "description": "Optional TTS provider override (`cloud` or `piper`)." },
                "voice": { "type": "string", "description": "Optional voice id for the chosen provider." },
                "format": { "type": "string", "enum": ["mp3", "wav"], "description": "Desired audio format." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(anyhow::Error::msg)?;
        let request: AudioGenerateRequest = serde_json::from_value(args)?;
        let outcome = generate_podcast(&self.config, request)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &outcome.value,
        )?))
    }
}

#[async_trait]
impl Tool for AudioEmailPodcastTool {
    fn name(&self) -> &str {
        "audio_email_podcast"
    }

    fn description(&self) -> &str {
        "Email a workspace audio file as an attachment so the recipient can listen to it like a podcast episode."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["to", "subject", "body", "audio_path"],
            "properties": {
                "to": { "type": "string", "description": "Recipient email address." },
                "subject": { "type": "string", "description": "Email subject line." },
                "body": { "type": "string", "description": "Email body text." },
                "audio_path": { "type": "string", "description": "Workspace-relative path to the generated audio file." },
                "attachment_name": { "type": "string", "description": "Optional attachment file name override." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(anyhow::Error::msg)?;
        let request: EmailPodcastRequest = serde_json::from_value(args)?;
        let outcome = email_podcast(&self.config, request)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &outcome.value,
        )?))
    }
}

#[async_trait]
impl Tool for AudioGenerateAndEmailPodcastTool {
    fn name(&self) -> &str {
        "audio_generate_and_email_podcast"
    }

    fn description(&self) -> &str {
        "Generate an audio file from text and immediately email it as a podcast-style attachment."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["text", "to", "subject", "body"],
            "properties": {
                "text": { "type": "string", "description": "Text to synthesize into audio." },
                "to": { "type": "string", "description": "Recipient email address." },
                "subject": { "type": "string", "description": "Email subject line." },
                "body": { "type": "string", "description": "Email body text." },
                "title": { "type": "string", "description": "Optional title used in the default file name." },
                "output_path": { "type": "string", "description": "Optional workspace-relative output path." },
                "provider": { "type": "string", "description": "Optional TTS provider override (`cloud` or `piper`)." },
                "voice": { "type": "string", "description": "Optional voice id for the chosen provider." },
                "format": { "type": "string", "enum": ["mp3", "wav"], "description": "Desired audio format." },
                "attachment_name": { "type": "string", "description": "Optional attachment file name override." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(anyhow::Error::msg)?;

        let text = required_string(&args, "text")?;
        let to = required_string(&args, "to")?;
        let subject = required_string(&args, "subject")?;
        let body = required_string(&args, "body")?;

        let generated = AudioGenerateRequest {
            text,
            title: optional_string(&args, "title"),
            output_path: optional_string(&args, "output_path"),
            provider: optional_string(&args, "provider"),
            voice: optional_string(&args, "voice"),
            format: optional_format(&args, "format")?,
        };
        let email = EmailPodcastRequest {
            to,
            subject,
            body,
            audio_path: String::new(),
            attachment_name: optional_string(&args, "attachment_name"),
        };
        let outcome = generate_and_email_podcast(&self.config, generated, email)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &outcome.value,
        )?))
    }
}

fn required_string(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required `{key}`"))
}

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn optional_format(
    args: &serde_json::Value,
    key: &str,
) -> anyhow::Result<Option<crate::openhuman::audio_toolkit::AudioFormat>> {
    let Some(raw) = args.get(key).and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    match raw.trim() {
        "mp3" => Ok(Some(crate::openhuman::audio_toolkit::AudioFormat::Mp3)),
        "wav" => Ok(Some(crate::openhuman::audio_toolkit::AudioFormat::Wav)),
        other => Err(anyhow::anyhow!("invalid format `{other}`")),
    }
}
