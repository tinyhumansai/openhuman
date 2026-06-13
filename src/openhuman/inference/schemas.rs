use serde::de::{DeserializeOwned, Deserializer};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct InferenceSummarizeParams {
    text: String,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct InferencePromptParams {
    prompt: String,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct InferenceVisionPromptParams {
    prompt: String,
    image_refs: Vec<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct InferenceResolveModelParams {
    hint: String,
}

#[derive(Debug, Deserialize)]
struct InferenceTestProviderModelParams {
    workload: String,
    provider: String,
    prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InferenceShouldReactParams {
    message: String,
    channel_type: String,
}

#[derive(Debug, Deserialize)]
struct InferenceAnalyzeSentimentParams {
    message: String,
}

#[derive(Debug, Deserialize)]
struct InferenceModelRouteUpdate {
    hint: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct InferenceCloudProviderUpdate {
    id: Option<String>,
    slug: String,
    #[serde(default)]
    label: Option<String>,
    endpoint: String,
    #[serde(default)]
    auth_style: Option<String>,
    #[serde(rename = "type", default)]
    legacy_type: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InferenceUpdateModelSettingsParams {
    api_url: Option<String>,
    inference_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    default_temperature: Option<f64>,
    model_routes: Option<Vec<InferenceModelRouteUpdate>>,
    cloud_providers: Option<Vec<InferenceCloudProviderUpdate>>,
    #[serde(default)]
    model_registry: Option<Vec<crate::openhuman::config::schema::ModelRegistryEntry>>,
    primary_cloud: Option<String>,
    chat_provider: Option<String>,
    reasoning_provider: Option<String>,
    agentic_provider: Option<String>,
    coding_provider: Option<String>,
    memory_provider: Option<String>,
    embeddings_provider: Option<String>,
    heartbeat_provider: Option<String>,
    learning_provider: Option<String>,
    subconscious_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InferenceUpdateLocalSettingsParams {
    runtime_enabled: Option<bool>,
    opt_in_confirmed: Option<bool>,
    provider: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_json")]
    base_url: Option<Value>,
    model_id: Option<String>,
    chat_model_id: Option<String>,
    usage_embeddings: Option<bool>,
    usage_heartbeat: Option<bool>,
    usage_learning_reflection: Option<bool>,
    usage_subconscious: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct InferenceListModelsParams {
    provider_id: String,
}

#[derive(Debug, Deserialize)]
struct InferenceApplyPresetParams {
    tier: String,
}

#[derive(Debug, Deserialize)]
struct InferenceOpenAiOAuthCompleteParams {
    #[serde(alias = "callbackUrl")]
    callback_url: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("resolve_model"),
        schemas("status"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_local_settings"),
        schemas("list_models"),
        schemas("device_profile"),
        schemas("presets"),
        schemas("apply_preset"),
        schemas("diagnostics"),
        schemas("openai_oauth_start"),
        schemas("openai_oauth_complete"),
        schemas("openai_oauth_import_codex_cli"),
        schemas("openai_oauth_status"),
        schemas("openai_oauth_disconnect"),
        schemas("summarize"),
        schemas("prompt"),
        schemas("vision_prompt"),
        schemas("test_provider_model"),
        schemas("should_react"),
        schemas("analyze_sentiment"),
        schemas("claude_code_status"),
        schemas("claude_code_auth_status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("resolve_model"),
            handler: handle_inference_resolve_model,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_inference_status,
        },
        RegisteredController {
            schema: schemas("get_client_config"),
            handler: handle_inference_get_client_config,
        },
        RegisteredController {
            schema: schemas("update_model_settings"),
            handler: handle_inference_update_model_settings,
        },
        RegisteredController {
            schema: schemas("update_local_settings"),
            handler: handle_inference_update_local_settings,
        },
        RegisteredController {
            schema: schemas("list_models"),
            handler: handle_inference_list_models,
        },
        RegisteredController {
            schema: schemas("device_profile"),
            handler: handle_inference_device_profile,
        },
        RegisteredController {
            schema: schemas("presets"),
            handler: handle_inference_presets,
        },
        RegisteredController {
            schema: schemas("apply_preset"),
            handler: handle_inference_apply_preset,
        },
        RegisteredController {
            schema: schemas("diagnostics"),
            handler: handle_inference_diagnostics,
        },
        RegisteredController {
            schema: schemas("openai_oauth_start"),
            handler: handle_inference_openai_oauth_start,
        },
        RegisteredController {
            schema: schemas("openai_oauth_complete"),
            handler: handle_inference_openai_oauth_complete,
        },
        RegisteredController {
            schema: schemas("openai_oauth_import_codex_cli"),
            handler: handle_inference_openai_oauth_import_codex_cli,
        },
        RegisteredController {
            schema: schemas("openai_oauth_status"),
            handler: handle_inference_openai_oauth_status,
        },
        RegisteredController {
            schema: schemas("openai_oauth_disconnect"),
            handler: handle_inference_openai_oauth_disconnect,
        },
        RegisteredController {
            schema: schemas("summarize"),
            handler: handle_inference_summarize,
        },
        RegisteredController {
            schema: schemas("prompt"),
            handler: handle_inference_prompt,
        },
        RegisteredController {
            schema: schemas("vision_prompt"),
            handler: handle_inference_vision_prompt,
        },
        RegisteredController {
            schema: schemas("test_provider_model"),
            handler: handle_inference_test_provider_model,
        },
        RegisteredController {
            schema: schemas("should_react"),
            handler: handle_inference_should_react,
        },
        RegisteredController {
            schema: schemas("analyze_sentiment"),
            handler: handle_inference_analyze_sentiment,
        },
        RegisteredController {
            schema: schemas("claude_code_status"),
            handler: handle_inference_claude_code_status,
        },
        RegisteredController {
            schema: schemas("claude_code_auth_status"),
            handler: handle_inference_claude_code_auth_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "resolve_model" => ControllerSchema {
            namespace: "inference",
            function: "resolve_model",
            description: "Resolve a model hint or tier name to the concrete model the provider router would use.",
            inputs: vec![required_string("hint", "Model hint (e.g. hint:reasoning) or tier name (e.g. reasoning-v1).")],
            outputs: vec![
                json_output("model", "Resolved concrete model id."),
                json_output(
                    "vision",
                    "Whether the resolved model accepts image input (vision-capable).",
                ),
            ],
        },
        "status" => ControllerSchema {
            namespace: "inference",
            function: "status",
            description: "Read inference service status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Inference status payload.")],
        },
        "get_client_config" => ControllerSchema {
            namespace: "inference",
            function: "get_client_config",
            description: "Read the client-facing inference/provider config used by the AI settings UI.",
            inputs: vec![],
            outputs: vec![json_output("config", "Client-facing inference config payload.")],
        },
        "update_model_settings" => ControllerSchema {
            namespace: "inference",
            function: "update_model_settings",
            description: "Persist cloud-provider routing, custom inference endpoint, and per-workload provider settings.",
            inputs: vec![
                optional_string("api_url", "Optional OpenHuman product backend URL."),
                optional_string("inference_url", "Optional custom inference base URL."),
                optional_string("api_key", "Optional API key for a custom inference endpoint."),
                optional_string("default_model", "Optional default model override."),
                optional_f64("default_temperature", "Optional default temperature override."),
                optional_json("model_routes", "Optional full replacement for legacy model routes."),
                optional_json("cloud_providers", "Optional full replacement for configured cloud providers."),
                optional_json("model_registry", "Optional full replacement for the per-model registry (carries each model's `vision` flag)."),
                optional_string("primary_cloud", "Optional primary cloud provider id."),
                optional_string("chat_provider", "Optional chat workload provider string."),
                optional_string("reasoning_provider", "Optional reasoning workload provider string."),
                optional_string("agentic_provider", "Optional agentic workload provider string."),
                optional_string("coding_provider", "Optional coding workload provider string."),
                optional_string("memory_provider", "Optional memory workload provider string."),
                optional_string("embeddings_provider", "Optional embeddings workload provider string."),
                optional_string("heartbeat_provider", "Optional heartbeat workload provider string."),
                optional_string("learning_provider", "Optional learning workload provider string."),
                optional_string("subconscious_provider", "Optional subconscious workload provider string."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_local_settings" => ControllerSchema {
            namespace: "inference",
            function: "update_local_settings",
            description: "Persist local inference provider selection, endpoint URL, and local-runtime routing flags.",
            inputs: vec![
                optional_bool("runtime_enabled", "Enable or disable local inference runtime routing."),
                optional_bool("opt_in_confirmed", "Persist the local inference opt-in flag."),
                optional_string("provider", "Optional local provider slug, e.g. ollama or lm_studio."),
                optional_json(
                    "base_url",
                    "Optional local provider base URL string, or null to clear.",
                ),
                optional_string("model_id", "Optional generic model id override."),
                optional_string("chat_model_id", "Optional chat model id override."),
                optional_bool("usage_embeddings", "Whether embeddings workload may use the local provider."),
                optional_bool("usage_heartbeat", "Whether heartbeat workload may use the local provider."),
                optional_bool("usage_learning_reflection", "Whether learning reflection workload may use the local provider."),
                optional_bool("usage_subconscious", "Whether subconscious workload may use the local provider."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "list_models" => ControllerSchema {
            namespace: "inference",
            function: "list_models",
            description: "Fetch the available model list from a configured inference provider's /models API.",
            inputs: vec![required_string("provider_id", "Opaque id of the cloud provider entry to query.")],
            outputs: vec![json_output("models", "Provider model list payload.")],
        },
        "device_profile" => ControllerSchema {
            namespace: "inference",
            function: "device_profile",
            description: "Detect the local hardware profile used for local inference recommendations.",
            inputs: vec![],
            outputs: vec![json_output("profile", "Device hardware profile.")],
        },
        "presets" => ControllerSchema {
            namespace: "inference",
            function: "presets",
            description: "List local inference model presets with recommendation and current selection.",
            inputs: vec![],
            outputs: vec![json_output("presets", "Inference preset payload.")],
        },
        "apply_preset" => ControllerSchema {
            namespace: "inference",
            function: "apply_preset",
            description: "Apply a local inference preset to the persisted config.",
            inputs: vec![required_string("tier", "Tier to apply: ram_2_4gb or disabled.")],
            outputs: vec![json_output("result", "Applied preset payload.")],
        },
        "diagnostics" => ControllerSchema {
            namespace: "inference",
            function: "diagnostics",
            description: "Run diagnostics for the configured local inference provider endpoint and expected models.",
            inputs: vec![],
            outputs: vec![json_output(
                "diagnostics",
                "Inference diagnostics payload. `installed_models[]` carries \
                 `context_length` and an `eligibility` verdict ({status: ok | \
                 below_minimum | unknown}); `context_requirement.min_context_tokens` \
                 is the memory-layer floor; `expected.{chat,embedding}_eligibility` \
                 mirror it for the active models. Models below the floor are rejected \
                 via `issues`.",
            )],
        },
        "openai_oauth_start" => ControllerSchema {
            namespace: "inference",
            function: "openai_oauth_start",
            description: "Begin ChatGPT/Codex OAuth (PKCE) for the openai cloud provider.",
            inputs: vec![],
            outputs: vec![json_output("result", "OAuth start payload with authUrl.")],
        },
        "openai_oauth_complete" => ControllerSchema {
            namespace: "inference",
            function: "openai_oauth_complete",
            description: "Complete ChatGPT/Codex OAuth using the browser callback URL.",
            inputs: vec![required_string(
                "callback_url",
                "Redirect URL after sign-in (http://127.0.0.1:1455/auth/callback?...).",
            )],
            outputs: vec![json_output("result", "OAuth completion payload.")],
        },
        "openai_oauth_import_codex_cli" => ControllerSchema {
            namespace: "inference",
            function: "openai_oauth_import_codex_cli",
            description: "Import the existing Codex CLI ChatGPT login from ~/.codex/auth.json.",
            inputs: vec![],
            outputs: vec![json_output("result", "OAuth import payload.")],
        },
        "openai_oauth_status" => ControllerSchema {
            namespace: "inference",
            function: "openai_oauth_status",
            description: "Whether ChatGPT OAuth credentials are stored for openai.",
            inputs: vec![],
            outputs: vec![json_output("status", "OAuth connection status.")],
        },
        "openai_oauth_disconnect" => ControllerSchema {
            namespace: "inference",
            function: "openai_oauth_disconnect",
            description: "Remove stored ChatGPT OAuth credentials.",
            inputs: vec![],
            outputs: vec![json_output("result", "Disconnect result.")],
        },
        "summarize" => ControllerSchema {
            namespace: "inference",
            function: "summarize",
            description: "Summarize text with the configured inference provider.",
            inputs: vec![
                required_string("text", "Input text."),
                optional_u64("max_tokens", "Optional max output tokens."),
            ],
            outputs: vec![json_output("summary", "Summary text.")],
        },
        "prompt" => ControllerSchema {
            namespace: "inference",
            function: "prompt",
            description: "Run a direct inference prompt.",
            inputs: vec![
                required_string("prompt", "Prompt text."),
                optional_u64("max_tokens", "Optional max output tokens."),
                optional_bool("no_think", "Disable thinking mode."),
            ],
            outputs: vec![json_output("output", "Prompt output text.")],
        },
        "vision_prompt" => ControllerSchema {
            namespace: "inference",
            function: "vision_prompt",
            description: "Run a multimodal inference prompt with image refs.",
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
        "test_provider_model" => ControllerSchema {
            namespace: "inference",
            function: "test_provider_model",
            description: "Run a one-off Hello-world style test against an explicit provider:model binding without saving routing changes.",
            inputs: vec![
                required_string("workload", "Workload id context (chat, reasoning, coding, etc.)."),
                required_string("provider", "Explicit provider string like 'openai:gpt-4o' or 'ollama:llama3.1:8b'."),
                optional_string("prompt", "Optional prompt text to send; defaults to 'Hello world'."),
            ],
            outputs: vec![json_output("reply", "Assistant reply text.")],
        },
        "should_react" => ControllerSchema {
            namespace: "inference",
            function: "should_react",
            description: "Ask the inference provider whether the assistant should add an emoji reaction to a user message, based on channel type.",
            inputs: vec![
                required_string("message", "User message content to evaluate."),
                required_string("channel_type", "Channel type: web, telegram, discord, slack, etc."),
            ],
            outputs: vec![json_output("decision", "Reaction decision: {should_react, emoji}.")],
        },
        "analyze_sentiment" => ControllerSchema {
            namespace: "inference",
            function: "analyze_sentiment",
            description: "Classify the emotion and valence of a user message with the inference provider.",
            inputs: vec![required_string("message", "User message content to classify.")],
            outputs: vec![json_output("sentiment", "Sentiment analysis payload.")],
        },
        "claude_code_status" => ControllerSchema {
            namespace: "inference",
            function: "claude_code_status",
            description: "Probe the local `claude` CLI binary (Claude Code CLI provider) and return install + version status.",
            inputs: vec![],
            outputs: vec![json_output(
                "status",
                "CliStatus payload: ok | not_installed | outdated | unusable, with version + path when present.",
            )],
        },
        "claude_code_auth_status" => ControllerSchema {
            namespace: "inference",
            function: "claude_code_auth_status",
            description: "Detect Claude Code CLI auth state (Pro/Max subscription via credentials.json, API key env, or none). No CLI spawn, no token round-trip.",
            inputs: vec![],
            outputs: vec![json_output(
                "auth",
                "AuthStatus payload: source = subscription | api_key_env | none, plus optional account_email + expires_at + last_checked.",
            )],
        },
        other => panic!("unknown inference schema: {other}"),
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
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

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
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

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
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

fn handle_inference_resolve_model(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceResolveModelParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let resolved = crate::openhuman::inference::provider::factory::resolve_model_for_hint(
            &p.hint, &config,
        );
        // Whether the resolved model accepts image input — drives the chat UI's
        // image-attachment affordance. Managed OpenHuman tiers consult the
        // core-owned per-tier map (currently all `false`); custom/BYOK models are
        // covered by the user's per-model `model_registry.vision` flag.
        let vision =
            crate::openhuman::inference::model_context::model_supports_vision(&resolved, &config);
        to_json(RpcOutcome::new(
            serde_json::json!({ "model": resolved, "vision": vision }),
            vec![],
        ))
    })
}

fn handle_inference_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_status(&config).await?)
    })
}

fn handle_inference_get_client_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::openhuman::inference::rpc::inference_get_client_config().await?)
    })
}

fn handle_inference_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateModelSettingsParams>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            inference_url: update.inference_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|route| crate::openhuman::config::ModelRouteConfig {
                        hint: route.hint,
                        model: route.model,
                    })
                    .collect()
            }),
            cloud_providers: update
                .cloud_providers
                .map(|entries| {
                    use crate::openhuman::config::schema::cloud_providers::{
                        generate_provider_id, is_slug_reserved, migrate_legacy_fields, AuthStyle,
                        CloudProviderCreds,
                    };
                    let reserved_count = entries
                        .iter()
                        .filter(|e| {
                            let t = e.slug.trim();
                            !t.is_empty() && is_slug_reserved(t)
                        })
                        .count();
                    if reserved_count > 0 {
                        log::debug!(
                            "[inference] update_model_settings: dropping {} reserved cloud provider slug(s)",
                            reserved_count
                        );
                    }
                    entries
                        .into_iter()
                        // Silently drop entries whose (non-empty) slug is reserved —
                        // typically the migration-seeded "openhuman" / "cloud" /
                        // "pid" built-ins that the frontend echoes back on every
                        // save (see `migrations::unify_ai_provider_settings`).
                        // Empty slugs still fall through so the explicit
                        // validation error below fires for actual frontend
                        // bugs. `apply_model_settings` re-injects the existing
                        // reserved entries from the stored config so they
                        // aren't dropped on save.
                        .filter(|entry| {
                            let trimmed = entry.slug.trim();
                            trimmed.is_empty() || !is_slug_reserved(trimmed)
                        })
                        .map(|entry| {
                            let slug = entry.slug.trim().to_string();
                            if slug.is_empty() {
                                return Err("cloud provider slug must not be empty".to_string());
                            }
                            let auth_style = match entry
                                .auth_style
                                .as_deref()
                                .unwrap_or("bearer")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "bearer" => AuthStyle::Bearer,
                                "anthropic" => AuthStyle::Anthropic,
                                "openhuman_jwt" | "openhumanjwt" => AuthStyle::OpenhumanJwt,
                                "none" => AuthStyle::None,
                                other => {
                                    return Err(format!(
                                        "unknown auth_style '{}'; valid: bearer, anthropic, openhuman_jwt, none",
                                        other
                                    ))
                                }
                            };
                            let id = entry
                                .id
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| generate_provider_id(&slug));
                            let label = entry
                                .label
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| slug.clone());
                            let mut provider = CloudProviderCreds {
                                id,
                                slug,
                                label,
                                endpoint: entry.endpoint,
                                auth_style,
                                legacy_type: entry.legacy_type,
                                default_model: entry.default_model,
                            };
                            migrate_legacy_fields(&mut provider);
                            Ok(provider)
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?,
            model_registry: update.model_registry,
            primary_cloud: update.primary_cloud,
            chat_provider: update.chat_provider,
            reasoning_provider: update.reasoning_provider,
            agentic_provider: update.agentic_provider,
            coding_provider: update.coding_provider,
            memory_provider: update.memory_provider,
            embeddings_provider: update.embeddings_provider,
            heartbeat_provider: update.heartbeat_provider,
            learning_provider: update.learning_provider,
            subconscious_provider: update.subconscious_provider,
        };
        to_json(crate::openhuman::inference::rpc::inference_update_model_settings(patch).await?)
    })
}

fn handle_inference_update_local_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateLocalSettingsParams>(params)?;
        let base_url = match update.base_url {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(value)) => Some(Some(value)),
            Some(_) => return Err("invalid params: base_url must be a string or null".to_string()),
        };
        let patch = config_rpc::LocalAiSettingsPatch {
            runtime_enabled: update.runtime_enabled,
            opt_in_confirmed: update.opt_in_confirmed,
            provider: update.provider,
            base_url,
            model_id: update.model_id,
            chat_model_id: update.chat_model_id,
            usage_embeddings: update.usage_embeddings,
            usage_heartbeat: update.usage_heartbeat,
            usage_learning_reflection: update.usage_learning_reflection,
            usage_subconscious: update.usage_subconscious,
        };
        to_json(crate::openhuman::inference::rpc::inference_update_local_settings(patch).await?)
    })
}

fn handle_inference_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request = deserialize_params::<InferenceListModelsParams>(params)?;
        to_json(
            crate::openhuman::inference::rpc::inference_list_models(&request.provider_id).await?,
        )
    })
}

fn handle_inference_device_profile(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(
        async move { to_json(crate::openhuman::inference::rpc::inference_device_profile().await?) },
    )
}

fn handle_inference_presets(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(crate::openhuman::inference::rpc::inference_presets().await?) })
}

fn handle_inference_apply_preset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request = deserialize_params::<InferenceApplyPresetParams>(params)?;
        to_json(crate::openhuman::inference::rpc::inference_apply_preset(&request.tier).await?)
    })
}

fn handle_inference_openai_oauth_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_start(&config).await?)
    })
}

fn handle_inference_openai_oauth_complete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<InferenceOpenAiOAuthCompleteParams>(params)?;
        to_json(
            crate::openhuman::inference::rpc::inference_openai_oauth_complete(
                &config,
                payload.callback_url.trim(),
            )
            .await?,
        )
    })
}

fn handle_inference_openai_oauth_import_codex_cli(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_openai_oauth_import_codex_cli(&config)
                .await?,
        )
    })
}

fn handle_inference_openai_oauth_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_status(&config).await?)
    })
}

fn handle_inference_openai_oauth_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_openai_oauth_disconnect(&config).await?)
    })
}

fn handle_inference_diagnostics(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::rpc::inference_diagnostics(&config).await?)
    })
}

fn handle_inference_summarize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceSummarizeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_summarize(&config, &p.text, p.max_tokens)
                .await?,
        )
    })
}

fn handle_inference_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferencePromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_prompt(
                &config,
                &p.prompt,
                p.max_tokens,
                p.no_think,
            )
            .await?,
        )
    })
}

fn handle_inference_vision_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceVisionPromptParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_vision_prompt(
                &config,
                &p.prompt,
                &p.image_refs,
                p.max_tokens,
            )
            .await?,
        )
    })
}

fn handle_inference_test_provider_model(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceTestProviderModelParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_test_provider_model(
                &config,
                &p.workload,
                &p.provider,
                p.prompt.as_deref().unwrap_or("Hello world"),
            )
            .await?,
        )
    })
}

fn handle_inference_should_react(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceShouldReactParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_should_react(
                &config,
                &p.message,
                &p.channel_type,
            )
            .await?,
        )
    })
}

fn handle_inference_analyze_sentiment(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceAnalyzeSentimentParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::rpc::inference_analyze_sentiment(&config, &p.message)
                .await?,
        )
    })
}

fn handle_inference_claude_code_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let status = tokio::task::spawn_blocking(
            crate::openhuman::inference::provider::claude_code::version_check::probe,
        )
        .await
        .map_err(|e| format!("claude_code_status join error: {e}"))?;
        to_json(RpcOutcome::new(status, vec![]))
    })
}

fn handle_inference_claude_code_auth_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let auth = tokio::task::spawn_blocking(
            crate::openhuman::inference::provider::claude_code::auth_status::probe,
        )
        .await
        .map_err(|e| format!("claude_code_auth_status join error: {e}"))?;
        to_json(RpcOutcome::new(auth, vec![]))
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn deserialize_present_json<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
