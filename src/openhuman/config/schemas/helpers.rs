use serde::de::{DeserializeOwned, Deserializer};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::{FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub(super) const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

#[derive(Debug, Deserialize)]
pub(super) struct ModelRouteUpdate {
    pub(super) hint: String,
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CloudProviderUpdate {
    /// Opaque stable id. Empty / missing → server generates a new id.
    pub(super) id: Option<String>,
    /// Routing slug, e.g. "openai", "my-deepseek". Must be unique per config.
    pub(super) slug: String,
    /// Human-readable label.
    #[serde(default)]
    pub(super) label: Option<String>,
    pub(super) endpoint: String,
    /// Auth style: "bearer" | "anthropic" | "openhuman_jwt" | "none".
    #[serde(default)]
    pub(super) auth_style: Option<String>,
    /// Legacy field — tolerated on read for back-compat but not required.
    #[serde(rename = "type", default)]
    pub(super) legacy_type: Option<String>,
    /// Legacy field — tolerated on read.
    #[serde(default)]
    pub(super) default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelSettingsUpdate {
    /// OpenHuman product backend URL. Used for auth, billing, voice, and
    /// every non-inference HTTP call. Almost always left blank so it
    /// defaults to the canonical hosted backend.
    pub(super) api_url: Option<String>,
    /// Custom OpenAI-compatible LLM endpoint. When set together with
    /// `api_key`, inference talks directly to this URL instead of routing
    /// through the OpenHuman backend. Send an empty string to clear.
    pub(super) inference_url: Option<String>,
    /// Optional API key for OpenAI-compatible backends. Stored verbatim in
    /// `config.toml` on the user's machine — see #1342 (local-first / pluggable
    /// backends). The key is never echoed back over RPC; `get_client_config`
    /// only reports `api_key_set: bool`.
    pub(super) api_key: Option<String>,
    pub(super) default_model: Option<String>,
    pub(super) default_temperature: Option<f64>,
    /// When present, REPLACES `config.model_routes` wholesale with these
    /// `(hint, model)` pairs. Send `Some([])` to clear all routes (used when
    /// the user switches back to the OpenHuman backend whose built-in router
    /// picks per-task models on its own). Omit to leave existing routes
    /// untouched.
    pub(super) model_routes: Option<Vec<ModelRouteUpdate>>,
    /// When present, REPLACES `config.cloud_providers` wholesale. The keys
    /// themselves live in `auth-profiles.json` via
    /// `cloud_provider_set_key` — they are NOT carried here.
    pub(super) cloud_providers: Option<Vec<CloudProviderUpdate>>,
    pub(super) primary_cloud: Option<String>,
    pub(super) chat_provider: Option<String>,
    pub(super) reasoning_provider: Option<String>,
    pub(super) agentic_provider: Option<String>,
    pub(super) coding_provider: Option<String>,
    pub(super) vision_provider: Option<String>,
    pub(super) memory_provider: Option<String>,
    pub(super) embeddings_provider: Option<String>,
    pub(super) heartbeat_provider: Option<String>,
    pub(super) learning_provider: Option<String>,
    pub(super) subconscious_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MemorySettingsUpdate {
    pub(super) backend: Option<String>,
    pub(super) auto_save: Option<bool>,
    pub(super) embedding_provider: Option<String>,
    pub(super) embedding_model: Option<String>,
    pub(super) embedding_dimensions: Option<usize>,
    /// One of `"minimal" | "balanced" | "extended" | "maximum"`.
    pub(super) memory_window: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RuntimeSettingsUpdate {
    pub(super) kind: Option<String>,
    pub(super) reasoning_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BrowserSettingsUpdate {
    pub(super) enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScreenIntelligenceSettingsUpdate {
    pub(super) enabled: Option<bool>,
    pub(super) capture_policy: Option<String>,
    pub(super) policy_mode: Option<String>,
    pub(super) baseline_fps: Option<f32>,
    pub(super) vision_enabled: Option<bool>,
    pub(super) autocomplete_enabled: Option<bool>,
    pub(super) use_vision_model: Option<bool>,
    pub(super) keep_screenshots: Option<bool>,
    pub(super) allowlist: Option<Vec<String>>,
    pub(super) denylist: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AnalyticsSettingsUpdate {
    pub(super) enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MeetSettingsUpdate {
    pub(super) auto_orchestrator_handoff: Option<bool>,
    /// Calendar auto-join policy as a string: `ask_each_time` | `always` | `never`.
    pub(super) auto_join_policy: Option<String>,
    /// Post-call summary policy as a string: `ask` | `always` | `never`.
    pub(super) auto_summarize_policy: Option<String>,
    pub(super) listen_only_default: Option<bool>,
    pub(super) ingest_backend_transcripts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchSettingsUpdate {
    pub(super) engine: Option<String>,
    pub(super) max_results: Option<usize>,
    pub(super) timeout_secs: Option<u64>,
    pub(super) parallel_api_key: Option<String>,
    pub(super) brave_api_key: Option<String>,
    pub(super) querit_api_key: Option<String>,
    pub(super) allowed_domains: Option<Vec<String>>,
    pub(super) allow_all: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LocalAiSettingsUpdate {
    pub(super) runtime_enabled: Option<bool>,
    /// MVP opt-in marker. Tied to `runtime_enabled` from the unified AI
    /// panel toggle (both flip on enable, both flip off on disable) so
    /// the user gets local AI working with a single click instead of
    /// having to also apply a tier preset.
    pub(super) opt_in_confirmed: Option<bool>,
    pub(super) provider: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_json")]
    pub(super) base_url: Option<Value>,
    pub(super) model_id: Option<String>,
    pub(super) chat_model_id: Option<String>,
    pub(super) usage_embeddings: Option<bool>,
    pub(super) usage_heartbeat: Option<bool>,
    pub(super) usage_learning_reflection: Option<bool>,
    pub(super) usage_subconscious: Option<bool>,
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetBrowserAllowAllParams {
    pub(super) enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkspaceOnboardingFlagParams {
    pub(super) flag_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkspaceOnboardingFlagSetParams {
    pub(super) flag_name: Option<String>,
    pub(super) value: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct OnboardingCompletedSetParams {
    pub(super) value: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SuperContextSetParams {
    pub(super) value: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct DictationSettingsUpdate {
    pub(super) enabled: Option<bool>,
    pub(super) hotkey: Option<String>,
    pub(super) activation_mode: Option<String>,
    pub(super) llm_refinement: Option<bool>,
    pub(super) streaming: Option<bool>,
    pub(super) streaming_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct VoiceServerSettingsUpdate {
    pub(super) auto_start: Option<bool>,
    pub(super) hotkey: Option<String>,
    pub(super) activation_mode: Option<String>,
    pub(super) skip_cleanup: Option<bool>,
    pub(super) min_duration_secs: Option<f32>,
    pub(super) silence_threshold: Option<f32>,
    pub(super) custom_dictionary: Option<Vec<String>>,
    pub(super) always_on_enabled: Option<bool>,
    pub(super) wake_word: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ComposioTriggerSettingsUpdate {
    pub(super) triage_disabled: Option<bool>,
    pub(super) triage_disabled_toolkits: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AutonomySettingsUpdate {
    /// `"readonly" | "supervised" | "full"` (case-insensitive).
    pub(super) level: Option<String>,
    pub(super) workspace_only: Option<bool>,
    /// Replaces the shell command allow-list wholesale.
    pub(super) allowed_commands: Option<Vec<String>>,
    /// Replaces the forbidden-paths denylist wholesale.
    pub(super) forbidden_paths: Option<Vec<String>>,
    /// Replaces the trusted-roots allow-list wholesale. Each entry is
    /// `{ "path": "/abs/dir", "access": "read" | "readwrite" }`.
    pub(super) trusted_roots: Option<Vec<crate::openhuman::security::TrustedRoot>>,
    pub(super) allow_tool_install: Option<bool>,
    // Accept u64 to match the published schema (`TypeSchema::U64`); clamped to the
    // internal u32 at apply time. u32::MAX/hr is already effectively unlimited.
    pub(super) max_actions_per_hour: Option<u64>,
    /// Replaces the "Always allow" allowlist wholesale — tool names the agent
    /// may run without an approval prompt. Empty list clears it.
    pub(super) auto_approve: Option<Vec<String>>,
    pub(super) require_task_plan_approval: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentSettingsUpdate {
    /// Tool/action wall-clock timeout in seconds (1–3600). Validated server-side.
    pub(super) agent_timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentPathsUpdate {
    /// New absolute action sandbox path. Empty string clears the override;
    /// omitted leaves it unchanged. Validated server-side.
    pub(super) action_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ActivityLevelSettingsUpdate {
    /// "off" | "minimal" | "moderate" | "active" | "always_on" (or "0"-"4").
    pub(super) level: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MemorySyncSettingsUpdate {
    pub(super) sync_interval_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SandboxSettingsUpdate {
    pub(super) backend: Option<String>,
    pub(super) enabled: Option<bool>,
    pub(super) docker_image: Option<String>,
    pub(super) docker_memory_limit_mb: Option<u64>,
    pub(super) docker_cpu_limit: Option<f64>,
    pub(super) env_passthrough: Option<Vec<String>>,
}

pub(super) fn deserialize_params<T: DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(super) fn deserialize_present_json<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

pub fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

pub fn optional_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
        comment,
        required: false,
    }
}

#[allow(dead_code)]
pub fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

pub fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

pub(super) fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
