use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::audio_toolkit::types::{
    AudioFormat, AudioGenerateRequest, EmailPodcastRequest,
};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct GenerateAndEmailParams {
    text: String,
    to: String,
    subject: String,
    body: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    format: Option<AudioFormat>,
    #[serde(default)]
    attachment_name: Option<String>,
}

pub fn all_audio_toolkit_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        audio_toolkit_schemas("generate_podcast"),
        audio_toolkit_schemas("email_podcast"),
        audio_toolkit_schemas("generate_and_email_podcast"),
    ]
}

pub fn all_audio_toolkit_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: audio_toolkit_schemas("generate_podcast"),
            handler: handle_generate_podcast,
        },
        RegisteredController {
            schema: audio_toolkit_schemas("email_podcast"),
            handler: handle_email_podcast,
        },
        RegisteredController {
            schema: audio_toolkit_schemas("generate_and_email_podcast"),
            handler: handle_generate_and_email_podcast,
        },
    ]
}

pub fn audio_toolkit_schemas(function: &str) -> ControllerSchema {
    match function {
        "generate_podcast" => ControllerSchema {
            namespace: "audio_toolkit",
            function: "generate_podcast",
            description: "Synthesize text into a workspace audio file for listen-later / podcast-style delivery.",
            inputs: vec![
                required_string("text", "Text to synthesize."),
                optional_string("title", "Optional title used to derive the default file name."),
                optional_string("output_path", "Optional workspace-relative output path."),
                optional_string("provider", "Optional TTS provider override (`cloud` or `piper`)."),
                optional_string("voice", "Optional provider-specific voice id."),
                FieldSchema {
                    name: "format",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional output format (`mp3` or `wav`).",
                    required: false,
                },
            ],
            outputs: vec![json_output("audio", "Generated audio artifact metadata.")],
        },
        "email_podcast" => ControllerSchema {
            namespace: "audio_toolkit",
            function: "email_podcast",
            description: "Email a previously generated workspace audio file as an attachment.",
            inputs: vec![
                required_string("to", "Destination email address."),
                required_string("subject", "Email subject line."),
                required_string("body", "Email body text."),
                required_string("audio_path", "Workspace-relative path to the audio attachment."),
                optional_string("attachment_name", "Optional attachment file name override."),
            ],
            outputs: vec![json_output("email", "Email delivery metadata.")],
        },
        "generate_and_email_podcast" => ControllerSchema {
            namespace: "audio_toolkit",
            function: "generate_and_email_podcast",
            description: "Generate an audio file from text and immediately email it as a podcast-style attachment.",
            inputs: vec![
                required_string("text", "Text to synthesize."),
                required_string("to", "Destination email address."),
                required_string("subject", "Email subject line."),
                required_string("body", "Email body text."),
                optional_string("title", "Optional title used for the generated file name."),
                optional_string("output_path", "Optional workspace-relative output path."),
                optional_string("provider", "Optional TTS provider override (`cloud` or `piper`)."),
                optional_string("voice", "Optional provider-specific voice id."),
                FieldSchema {
                    name: "format",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional output format (`mp3` or `wav`).",
                    required: false,
                },
                optional_string("attachment_name", "Optional attachment file name override."),
            ],
            outputs: vec![json_output("result", "Combined audio generation and email-delivery result.")],
        },
        _ => ControllerSchema {
            namespace: "audio_toolkit",
            function: "unknown",
            description: "Unknown audio toolkit controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_generate_podcast(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let request: AudioGenerateRequest =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(crate::openhuman::audio_toolkit::generate_podcast(&config, request).await?)
    })
}

fn handle_email_podcast(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let request: EmailPodcastRequest =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(crate::openhuman::audio_toolkit::email_podcast(&config, request).await?)
    })
}

fn handle_generate_and_email_podcast(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let request: GenerateAndEmailParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let generated = AudioGenerateRequest {
            text: request.text,
            title: request.title,
            output_path: request.output_path,
            provider: request.provider,
            voice: request.voice,
            format: request.format,
        };
        let email = EmailPodcastRequest {
            to: request.to,
            subject: request.subject,
            body: request.body,
            audio_path: String::new(),
            attachment_name: request.attachment_name,
        };
        to_json(
            crate::openhuman::audio_toolkit::generate_and_email_podcast(&config, generated, email)
                .await?,
        )
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    serde_json::to_value(outcome.value).map_err(|e| format!("serialize error: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}
