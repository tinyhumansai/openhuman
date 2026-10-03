//! Schemas for config snapshot reads and model, memory, runtime, and local-AI inference settings.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_json, optional_string};

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"get_config" => Some( ControllerSchema {
            namespace: "config",
            function: "get",
            description: "Read persisted config snapshot and resolved paths.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "snapshot",
                ty: TypeSchema::Json,
                comment: "Config snapshot with workspace and config paths.",
                required: true,
            }],
        }),
"get_client_config" => Some( ControllerSchema {
            namespace: "config",
            function: "get_client_config",
            description: "Read safe client-facing config fields (api_url, feature flags). No secrets.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "api_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Configured OpenHuman product backend URL, if any.",
                    required: false,
                },
                FieldSchema {
                    name: "inference_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Custom OpenAI-compatible LLM endpoint, if any. When set together with an api_key, inference goes direct to this URL.",
                    required: false,
                },
                FieldSchema {
                    name: "default_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Default model identifier.",
                    required: false,
                },
                FieldSchema {
                    name: "app_version",
                    ty: TypeSchema::String,
                    comment: "OpenHuman core version.",
                    required: true,
                },
                FieldSchema {
                    name: "api_key_set",
                    ty: TypeSchema::Bool,
                    comment: "True when a custom backend api_key is stored locally. The key itself is never returned over RPC.",
                    required: true,
                },
                FieldSchema {
                    name: "model_routes",
                    ty: TypeSchema::Json,
                    comment: "Persisted task-hint -> model id pairs the core router will obey. Empty when the OpenHuman built-in router is active.",
                    required: true,
                },
            ],
        }),
"update_model_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_model_settings",
            description: "Update model and backend connection settings, including a custom OpenAI-compatible backend (api_url + api_key).",
            inputs: vec![
                optional_string("api_url", "OpenHuman product backend URL (auth/billing/voice). Almost always left blank; the inference URL is a separate `inference_url` field."),
                optional_string("inference_url", "Custom OpenAI-compatible LLM endpoint. When set together with `api_key`, inference goes direct to this URL instead of the OpenHuman backend. Pass an empty string to clear."),
                optional_string("api_key", "Optional API key for the configured inference endpoint. Pass an empty string to clear a previously stored key."),
                optional_string("default_model", "Default model id."),
                FieldSchema {
                    name: "default_temperature",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Default model temperature.",
                    required: false,
                },
                FieldSchema {
                    name: "model_routes",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of {hint, model} pairs mapping task hints (reasoning, agentic, coding, summarization) to provider-specific model ids. Replaces config.model_routes wholesale; send [] to clear (e.g. when switching back to the OpenHuman built-in router).",
                    required: false,
                },
                FieldSchema {
                    name: "cloud_providers",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of cloud provider entries {id, slug, label, endpoint, auth_style}. API keys are stored separately via cloud_provider_set_key. Replaces config.cloud_providers wholesale.",
                    required: false,
                },
                optional_string("primary_cloud", "id of the cloud_providers entry used when a workload routes to 'cloud'. Empty string clears."),
                optional_string("chat_provider", "Provider string for direct conversational chat workloads."),
                optional_string("reasoning_provider", "Provider string for the main reasoning workload (e.g. 'cloud', 'ollama:llama3.1:8b', 'openai:gpt-4o')."),
                optional_string("agentic_provider", "Provider string for sub-agent / tool-loop workloads."),
                optional_string("coding_provider", "Provider string for code-generation workloads."),
                optional_string("vision_provider", "Provider string for the vision / multimodal workload (managed default: the default model)."),
                optional_string("memory_provider", "Provider string for summarisation."),
                optional_string("embeddings_provider", "Provider string for embedding generation."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"update_memory_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_memory_settings",
            description: "Update the embedding settings and the agent's memory-context window.",
            inputs: vec![
                optional_string("embedding_provider", "Embedding provider identifier."),
                optional_string("embedding_model", "Embedding model identifier."),
                FieldSchema {
                    name: "embedding_dimensions",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Embedding dimensions.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"update_runtime_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_runtime_settings",
            description: "Update runtime execution strategy settings.",
            inputs: vec![
                optional_string("kind", "Runtime kind."),
                optional_bool("reasoning_enabled", "Enable reasoning mode."),
                optional_string(
                    "reasoning_effort",
                    "Default reasoning effort for agent turns: none, minimal, low, medium, high or xhigh. Empty string clears it back to the provider default.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"update_local_ai_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_local_ai_settings",
            description:
                "Update the local AI runtime master switch and per-feature usage flags.",
            inputs: vec![
                optional_bool(
                    "runtime_enabled",
                    "Master switch — when false, no subsystem uses the selected local AI runtime.",
                ),
                optional_bool(
                    "opt_in_confirmed",
                    "MVP opt-in marker. Bootstrap hard-overrides to disabled when this is false, \
                     regardless of `runtime_enabled`. Set in tandem with `runtime_enabled` from the \
                     unified AI panel.",
                ),
                optional_string(
                    "provider",
                    "Local provider identifier. Supported values: ollama, lm_studio, omlx.",
                ),
                optional_json(
                    "base_url",
                    "Provider base URL string, or null to clear. For LM Studio this defaults to http://localhost:1234/v1.",
                ),
                optional_string(
                    "api_key",
                    "Bearer credential for keyed local runtimes such as OMLX. Pass an empty string to clear.",
                ),
                optional_string("model_id", "Default local chat model identifier."),
                optional_string("chat_model_id", "Local chat model identifier."),
                optional_bool(
                    "usage_embeddings",
                    "Use the local model for embedding generation (when runtime_enabled).",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"resolve_api_url" => Some( ControllerSchema {
            namespace: "config",
            function: "resolve_api_url",
            description: "Resolve effective API base URL using config/env/default from core.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "api_url",
                ty: TypeSchema::String,
                comment: "Resolved backend API URL.",
                required: true,
            }],
        }),
"get_runtime_flags" => Some( ControllerSchema {
            namespace: "config",
            function: "get_runtime_flags",
            description: "Read environment-driven runtime flags.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flags",
                ty: TypeSchema::Ref("RuntimeFlagsOut"),
                comment: "Runtime flag state.",
                required: true,
            }],
        }),
        _ => None,
    }
}
