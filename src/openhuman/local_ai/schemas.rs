use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionStartParams {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionControlParams {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiDownloadParams {
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSummarizeParams {
    text: String,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LocalAiPromptParams {
    prompt: String,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSuggestParams {
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    lines: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct LocalAiVisionPromptParams {
    prompt: String,
    image_refs: Vec<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LocalAiEmbedParams {
    inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeParams {
    audio_path: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeBytesParams {
    audio_bytes: Vec<u8>,
    extension: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTtsParams {
    text: String,
    output_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiDownloadAssetParams {
    capability: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiApplyPresetParams {
    tier: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiSetOllamaPathParams {
    path: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("agent_chat"),
        schemas("agent_chat_simple"),
        schemas("agent_repl_session_start"),
        schemas("agent_repl_session_reset"),
        schemas("agent_repl_session_end"),
        schemas("local_ai_status"),
        schemas("local_ai_download"),
        schemas("local_ai_download_all_assets"),
        schemas("local_ai_summarize"),
        schemas("local_ai_suggest_questions"),
        schemas("local_ai_prompt"),
        schemas("local_ai_vision_prompt"),
        schemas("local_ai_embed"),
        schemas("local_ai_transcribe"),
        schemas("local_ai_transcribe_bytes"),
        schemas("local_ai_tts"),
        schemas("local_ai_assets_status"),
        schemas("local_ai_downloads_progress"),
        schemas("local_ai_download_asset"),
        schemas("local_ai_device_profile"),
        schemas("local_ai_presets"),
        schemas("local_ai_apply_preset"),
        schemas("local_ai_set_ollama_path"),
        schemas("local_ai_diagnostics"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("agent_chat"),
            handler: handle_agent_chat,
        },
        RegisteredController {
            schema: schemas("agent_chat_simple"),
            handler: handle_agent_chat_simple,
        },
        RegisteredController {
            schema: schemas("agent_repl_session_start"),
            handler: handle_agent_repl_session_start,
        },
        RegisteredController {
            schema: schemas("agent_repl_session_reset"),
            handler: handle_agent_repl_session_reset,
        },
        RegisteredController {
            schema: schemas("agent_repl_session_end"),
            handler: handle_agent_repl_session_end,
        },
        RegisteredController {
            schema: schemas("local_ai_status"),
            handler: handle_local_ai_status,
        },
        RegisteredController {
            schema: schemas("local_ai_download"),
            handler: handle_local_ai_download,
        },
        RegisteredController {
            schema: schemas("local_ai_download_all_assets"),
            handler: handle_local_ai_download_all_assets,
        },
        RegisteredController {
            schema: schemas("local_ai_summarize"),
            handler: handle_local_ai_summarize,
        },
        RegisteredController {
            schema: schemas("local_ai_suggest_questions"),
            handler: handle_local_ai_suggest_questions,
        },
        RegisteredController {
            schema: schemas("local_ai_prompt"),
            handler: handle_local_ai_prompt,
        },
        RegisteredController {
            schema: schemas("local_ai_vision_prompt"),
            handler: handle_local_ai_vision_prompt,
        },
        RegisteredController {
            schema: schemas("local_ai_embed"),
            handler: handle_local_ai_embed,
        },
        RegisteredController {
            schema: schemas("local_ai_transcribe"),
            handler: handle_local_ai_transcribe,
        },
        RegisteredController {
            schema: schemas("local_ai_transcribe_bytes"),
            handler: handle_local_ai_transcribe_bytes,
        },
        RegisteredController {
            schema: schemas("local_ai_tts"),
            handler: handle_local_ai_tts,
        },
        RegisteredController {
            schema: schemas("local_ai_assets_status"),
            handler: handle_local_ai_assets_status,
        },
        RegisteredController {
            schema: schemas("local_ai_downloads_progress"),
            handler: handle_local_ai_downloads_progress,
        },
        RegisteredController {
            schema: schemas("local_ai_download_asset"),
            handler: handle_local_ai_download_asset,
        },
        RegisteredController {
            schema: schemas("local_ai_device_profile"),
            handler: handle_local_ai_device_profile,
        },
        RegisteredController {
            schema: schemas("local_ai_presets"),
            handler: handle_local_ai_presets,
        },
        RegisteredController {
            schema: schemas("local_ai_apply_preset"),
            handler: handle_local_ai_apply_preset,
        },
        RegisteredController {
            schema: schemas("local_ai_set_ollama_path"),
            handler: handle_local_ai_set_ollama_path,
        },
        RegisteredController {
            schema: schemas("local_ai_diagnostics"),
            handler: handle_local_ai_diagnostics,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "agent_chat" => ControllerSchema {
            namespace: "local_ai",
            function: "agent_chat",
            description: "Run one-shot agent chat with optional model overrides.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "agent_chat_simple" => ControllerSchema {
            namespace: "local_ai",
            function: "agent_chat_simple",
            description: "Run one-shot lightweight provider chat.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "agent_repl_session_start" => ControllerSchema {
            namespace: "local_ai",
            function: "agent_repl_session_start",
            description: "Create a persistent REPL agent session.",
            inputs: vec![
                optional_string("session_id", "Optional session id."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("result", "Session creation result.")],
        },
        "agent_repl_session_reset" => ControllerSchema {
            namespace: "local_ai",
            function: "agent_repl_session_reset",
            description: "Clear REPL session history.",
            inputs: vec![required_string("session_id", "REPL session id.")],
            outputs: vec![json_output("result", "Session reset result.")],
        },
        "agent_repl_session_end" => ControllerSchema {
            namespace: "local_ai",
            function: "agent_repl_session_end",
            description: "Terminate REPL session.",
            inputs: vec![required_string("session_id", "REPL session id.")],
            outputs: vec![json_output("result", "Session end result.")],
        },
        "local_ai_status" => ControllerSchema {
            namespace: "local_ai",
            function: "status",
            description: "Read local AI service status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Local AI status payload.")],
        },
        "local_ai_download" => ControllerSchema {
            namespace: "local_ai",
            function: "download",
            description: "Trigger local AI model download bootstrap.",
            inputs: vec![optional_bool("force", "Reset state before download.")],
            outputs: vec![json_output("status", "Local AI status payload.")],
        },
        "local_ai_download_all_assets" => ControllerSchema {
            namespace: "local_ai",
            function: "download_all_assets",
            description: "Trigger full local AI asset download.",
            inputs: vec![optional_bool("force", "Reset state before download.")],
            outputs: vec![json_output("progress", "Download progress payload.")],
        },
        "local_ai_summarize" => ControllerSchema {
            namespace: "local_ai",
            function: "summarize",
            description: "Summarize text with local AI model.",
            inputs: vec![
                required_string("text", "Input text."),
                optional_u64("max_tokens", "Optional max output tokens."),
            ],
            outputs: vec![json_output("summary", "Summary text.")],
        },
        "local_ai_suggest_questions" => ControllerSchema {
            namespace: "local_ai",
            function: "suggest_questions",
            description: "Suggest questions for provided context.",
            inputs: vec![
                optional_string("context", "Context text."),
                FieldSchema {
                    name: "lines",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Alternative context as lines.",
                    required: false,
                },
            ],
            outputs: vec![json_output("suggestions", "Suggested questions payload.")],
        },
        "local_ai_prompt" => ControllerSchema {
            namespace: "local_ai",
            function: "prompt",
            description: "Run direct local AI prompt.",
            inputs: vec![
                required_string("prompt", "Prompt text."),
                optional_u64("max_tokens", "Optional max output tokens."),
                optional_bool("no_think", "Disable thinking mode."),
            ],
            outputs: vec![json_output("output", "Prompt output text.")],
        },
        "local_ai_vision_prompt" => ControllerSchema {
            namespace: "local_ai",
            function: "vision_prompt",
            description: "Run multimodal local AI prompt with image refs.",
            inputs: vec![
                required_string("prompt", "Prompt text."),
                FieldSchema {
                    name: "image_refs",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Image references to include.",
                    required: true,
                },
                optional_u64("max_tokens", "Optional max output tokens."),
            ],
            outputs: vec![json_output("output", "Prompt output text.")],
        },
        "local_ai_embed" => ControllerSchema {
            namespace: "local_ai",
            function: "embed",
            description: "Generate embeddings for text inputs.",
            inputs: vec![FieldSchema {
                name: "inputs",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Texts to embed.",
                required: true,
            }],
            outputs: vec![json_output("embedding", "Embedding result payload.")],
        },
        "local_ai_transcribe" => ControllerSchema {
            namespace: "local_ai",
            function: "transcribe",
            description: "Transcribe audio from file path.",
            inputs: vec![required_string("audio_path", "Input audio path.")],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "local_ai_transcribe_bytes" => ControllerSchema {
            namespace: "local_ai",
            function: "transcribe_bytes",
            description: "Transcribe audio from raw bytes.",
            inputs: vec![
                FieldSchema {
                    name: "audio_bytes",
                    ty: TypeSchema::Bytes,
                    comment: "Raw audio bytes.",
                    required: true,
                },
                optional_string("extension", "Optional audio extension."),
            ],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "local_ai_tts" => ControllerSchema {
            namespace: "local_ai",
            function: "tts",
            description: "Synthesize speech from text.",
            inputs: vec![
                required_string("text", "Input text."),
                optional_string("output_path", "Optional output path."),
            ],
            outputs: vec![json_output("tts", "TTS result payload.")],
        },
        "local_ai_assets_status" => ControllerSchema {
            namespace: "local_ai",
            function: "assets_status",
            description: "Get local AI asset installation status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Assets status payload.")],
        },
        "local_ai_downloads_progress" => ControllerSchema {
            namespace: "local_ai",
            function: "downloads_progress",
            description: "Get local AI download progress.",
            inputs: vec![],
            outputs: vec![json_output("progress", "Download progress payload.")],
        },
        "local_ai_download_asset" => ControllerSchema {
            namespace: "local_ai",
            function: "download_asset",
            description: "Trigger download for one local AI asset capability.",
            inputs: vec![required_string("capability", "Asset capability id.")],
            outputs: vec![json_output("status", "Assets status payload.")],
        },
        "local_ai_device_profile" => ControllerSchema {
            namespace: "local_ai",
            function: "device_profile",
            description: "Detect local device hardware profile (RAM, CPU, GPU).",
            inputs: vec![],
            outputs: vec![json_output("profile", "Device hardware profile.")],
        },
        "local_ai_presets" => ControllerSchema {
            namespace: "local_ai",
            function: "presets",
            description: "List model tier presets with recommendation and current selection.",
            inputs: vec![],
            outputs: vec![json_output(
                "presets",
                "Presets, recommended tier, current tier.",
            )],
        },
        "local_ai_apply_preset" => ControllerSchema {
            namespace: "local_ai",
            function: "apply_preset",
            description: "Apply a model tier preset to local AI config and persist.",
            inputs: vec![required_string("tier", "Tier to apply: low, medium, high.")],
            outputs: vec![json_output("result", "Applied tier status.")],
        },
        "local_ai_diagnostics" => ControllerSchema {
            namespace: "local_ai",
            function: "diagnostics",
            description: "Run Ollama diagnostics: check server health, list installed models, verify expected models.",
            inputs: vec![],
            outputs: vec![json_output("diagnostics", "Diagnostic report.")],
        },
        "local_ai_set_ollama_path" => ControllerSchema {
            namespace: "local_ai",
            function: "set_ollama_path",
            description: "Set a custom Ollama binary path, persist to config, and trigger re-bootstrap.",
            inputs: vec![required_string("path", "Absolute path to Ollama binary. Empty string to clear.")],
            outputs: vec![json_output("result", "Updated status.")],
        },
        _ => ControllerSchema {
            namespace: "local_ai",
            function: "unknown",
            description: "Unknown local_ai controller function.",
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

fn handle_agent_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_agent_chat_simple(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat_simple(
                &config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_agent_repl_session_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionStartParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_repl_session_start(
                &config,
                p.session_id,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_agent_repl_session_reset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionControlParams>(params)?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_repl_session_reset(p.session_id.trim()).await?,
        )
    })
}

fn handle_agent_repl_session_end(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionControlParams>(params)?;
        to_json(crate::openhuman::local_ai::rpc::agent_repl_session_end(p.session_id.trim()).await?)
    })
}

fn handle_local_ai_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::local_ai::rpc::local_ai_status(&config).await?)
    })
}

fn handle_local_ai_download(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiDownloadParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_download(&config, p.force.unwrap_or(false))
                .await?,
        )
    })
}

fn handle_local_ai_download_all_assets(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiDownloadParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_download_all_assets(
                &config,
                p.force.unwrap_or(false),
            )
            .await?,
        )
    })
}

fn handle_local_ai_summarize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiSummarizeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_summarize(&config, &p.text, p.max_tokens)
                .await?,
        )
    })
}

fn handle_local_ai_suggest_questions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiSuggestParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_suggest_questions(
                &config, p.context, p.lines,
            )
            .await?,
        )
    })
}

fn handle_local_ai_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiPromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_prompt(
                &config,
                &p.prompt,
                p.max_tokens,
                p.no_think,
            )
            .await?,
        )
    })
}

fn handle_local_ai_vision_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiVisionPromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_vision_prompt(
                &config,
                &p.prompt,
                &p.image_refs,
                p.max_tokens,
            )
            .await?,
        )
    })
}

fn handle_local_ai_embed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiEmbedParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::local_ai::rpc::local_ai_embed(&config, &p.inputs).await?)
    })
}

fn handle_local_ai_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_transcribe(&config, p.audio_path.trim())
                .await?,
        )
    })
}

fn handle_local_ai_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeBytesParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
            )
            .await?,
        )
    })
}

fn handle_local_ai_tts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTtsParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_tts(
                &config,
                &p.text,
                p.output_path.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_local_ai_assets_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::local_ai::rpc::local_ai_assets_status(&config).await?)
    })
}

fn handle_local_ai_downloads_progress(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::local_ai::rpc::local_ai_downloads_progress(&config).await?)
    })
}

fn handle_local_ai_download_asset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiDownloadAssetParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::local_ai_download_asset(&config, p.capability.trim())
                .await?,
        )
    })
}

fn handle_local_ai_device_profile(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[local_ai] device_profile: detecting hardware");
        let profile = crate::openhuman::local_ai::device::detect_device_profile();
        tracing::debug!("[local_ai] device_profile: done");
        let value = serde_json::to_value(&profile).map_err(|e| format!("serialize: {e}"))?;
        Ok(value)
    })
}

fn handle_local_ai_presets(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[local_ai] presets: loading config and computing tiers");
        let config = config_rpc::load_config_with_timeout().await?;
        let device = crate::openhuman::local_ai::device::detect_device_profile();
        let recommended = crate::openhuman::local_ai::presets::recommend_tier(&device);
        let current =
            crate::openhuman::local_ai::presets::current_tier_from_config(&config.local_ai);
        let presets = crate::openhuman::local_ai::presets::all_presets();
        tracing::debug!(
            ?recommended,
            ?current,
            preset_count = presets.len(),
            "[local_ai] presets: returning"
        );
        let value = serde_json::json!({
            "presets": presets,
            "recommended_tier": recommended,
            "current_tier": current,
            "device": device,
        });
        Ok(value)
    })
}

fn handle_local_ai_apply_preset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiApplyPresetParams>(params)?;
        let tier_str = p.tier.trim().to_ascii_lowercase();
        tracing::debug!(tier = %tier_str, "[local_ai] apply_preset: parsing tier");

        let tier = crate::openhuman::local_ai::presets::ModelTier::from_str_opt(&tier_str)
            .ok_or_else(|| {
                format!(
                    "invalid tier '{}': expected one of low, medium, high",
                    tier_str
                )
            })?;

        if tier == crate::openhuman::local_ai::presets::ModelTier::Custom {
            return Err("cannot apply 'custom' tier; set model IDs directly".to_string());
        }

        let mut config = config_rpc::load_config_with_timeout().await?;
        crate::openhuman::local_ai::presets::apply_preset_to_config(&mut config.local_ai, tier);
        config
            .save()
            .await
            .map_err(|e| format!("save config: {e}"))?;
        tracing::debug!(tier = %tier_str, "[local_ai] apply_preset: config saved");

        Ok(serde_json::json!({
            "applied_tier": tier,
            "chat_model_id": config.local_ai.chat_model_id,
            "vision_model_id": config.local_ai.vision_model_id,
            "embedding_model_id": config.local_ai.embedding_model_id,
            "quantization": config.local_ai.quantization,
        }))
    })
}

fn handle_local_ai_diagnostics(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let service = crate::openhuman::local_ai::global(&config);
        service.diagnostics(&config).await
    })
}

fn handle_local_ai_set_ollama_path(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiSetOllamaPathParams>(params)?;
        let path_str = p.path.trim().to_string();
        tracing::debug!(path = %path_str, "[local_ai] set_ollama_path: validating");

        let new_value = if path_str.is_empty() {
            None
        } else {
            let path = std::path::Path::new(&path_str);
            if !path.is_file() {
                return Err(format!(
                    "Ollama binary not found at '{}'. Provide a valid path to the ollama executable.",
                    path_str
                ));
            }
            Some(path_str.clone())
        };

        let mut config = config_rpc::load_config_with_timeout().await?;
        config.local_ai.ollama_binary_path = new_value.clone();
        config
            .save()
            .await
            .map_err(|e| format!("save config: {e}"))?;
        tracing::debug!(path = ?new_value, "[local_ai] set_ollama_path: config saved, triggering re-bootstrap");

        let service = crate::openhuman::local_ai::global(&config);
        service.reset_to_idle(&config);
        let service_clone = service.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });

        let current_status = serde_json::to_value(&service.status())
            .map_err(|e| format!("serialize: {e}"))?;
        Ok(serde_json::json!({
            "ollama_binary_path": new_value,
            "status": current_status,
        }))
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
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

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
