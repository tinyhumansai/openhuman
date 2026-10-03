//! The `inference.*` controller schemas: one [`ControllerSchema`] per RPC
//! function, plus the field-shape helpers they are built from.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "resolve_model" => ControllerSchema {
            namespace: "inference",
            function: "resolve_model",
            description: "Resolve a model hint or tier name to the concrete model the provider router would use.",
            inputs: vec![required_string("hint", "Model hint (e.g. hint:reasoning) or a concrete model id.")],
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
                optional_string("vision_provider", "Optional vision / multimodal workload provider string."),
                optional_string("memory_provider", "Optional memory workload provider string."),
                optional_string("embeddings_provider", "Optional embeddings workload provider string."),
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
                optional_string(
                    "api_key",
                    "Optional Bearer API key for a local provider that requires one (e.g. OMLX); empty string clears it.",
                ),
                optional_string("model_id", "Optional generic model id override."),
                optional_string("chat_model_id", "Optional chat model id override."),
                optional_bool("usage_embeddings", "Whether embeddings workload may use the local provider."),
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
        "provider_auth_errors" => ControllerSchema {
            namespace: "inference",
            function: "provider_auth_errors",
            description: "List BYO provider auth failures (invalid/revoked key, 401/403) recorded this process, for the AI settings provider-error notice.",
            inputs: vec![],
            outputs: vec![json_output(
                "errors",
                "Array of {provider, status, message, timestamp_ms} provider auth errors.",
            )],
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
        "claude_code_settings" => ControllerSchema {
            namespace: "inference",
            function: "claude_code_settings",
            description: "Read the persisted Claude Code provider settings (currently just the full-access toggle). Self-contained per-install state kept in its own claude_code_settings.json beside config.toml in the OpenHuman config directory — not inside the central config file, and not under the internal workspace dir (config.workspace_dir).",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "ClaudeCodeSettings payload: { full_access: bool }. full_access=true → bypassPermissions + full toolset; false (default) → acceptEdits.",
            )],
        },
        "claude_code_set_full_access" => ControllerSchema {
            namespace: "inference",
            function: "claude_code_set_full_access",
            description: "Persist the Claude Code full-access toggle. true → bypassPermissions + full native toolset (Bash/network/subagents); false (default) → acceptEdits (file edits only). The OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE env var overrides this at runtime.",
            inputs: vec![required_bool(
                "enabled",
                "true → full access (bypassPermissions); false → acceptEdits.",
            )],
            outputs: vec![json_output(
                "settings",
                "The persisted ClaudeCodeSettings after the update: { full_access: bool }.",
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

fn required_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Bool,
        comment,
        required: true,
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
