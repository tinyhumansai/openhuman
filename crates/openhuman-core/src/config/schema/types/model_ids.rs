//! Standard model-tier identifiers and memory-sync cadence constants.

/// The managed backend's default model: DeepSeek V4 Flash through the
/// OpenRouter passthrough. Every managed workload runs on it unless the user
/// pins another catalog model (Settings → Routing → Default model, or a
/// per-agent / per-node pin).
///
/// This replaced the abstract `chat-v1` / `agentic-v1` / … tier slugs: the
/// backend no longer serves tier endpoints, only OpenRouter model ids. Workload
/// *roles* (`chat`, `reasoning`, `coding`, …) remain the internal vocabulary —
/// they choose the provider route and drive fallback and cost attribution —
/// but on the managed backend they all resolve to a concrete catalog id.
pub const MODEL_MANAGED_DEFAULT: &str = "openrouter/deepseek/deepseek-v4-flash";

/// Default model used when no explicit model is configured.
pub const DEFAULT_MODEL: &str = MODEL_MANAGED_DEFAULT;

/// The retired managed tier slugs, in the order they were introduced. Older
/// builds persisted them in `default_model`, the `*_provider` routes, agent
/// manifests and flow nodes; the config migration rewrites them to
/// [`MODEL_MANAGED_DEFAULT`], and the routing layer keeps accepting them as
/// aliases of their workload role so an un-migrated caller still routes.
pub const LEGACY_TIER_MODELS: [&str; 8] = [
    "chat-v1",
    "reasoning-v1",
    "reasoning-quick-v1",
    "agentic-v1",
    "coding-v1",
    "burst-v1",
    "summarization-v1",
    "vision-v1",
];

/// Whether `model` is one of the retired tier slugs ([`LEGACY_TIER_MODELS`]).
pub fn is_legacy_tier_model(model: &str) -> bool {
    legacy_tier_role(model).is_some()
}

/// The workload role a retired tier slug stood for, so a legacy value still
/// routes through the right `*_provider` field.
pub fn legacy_tier_role(model: &str) -> Option<&'static str> {
    match model.trim() {
        "chat-v1" | "reasoning-quick-v1" => Some("chat"),
        "reasoning-v1" => Some("reasoning"),
        "agentic-v1" => Some("agentic"),
        "coding-v1" => Some("coding"),
        "burst-v1" => Some("burst"),
        "summarization-v1" => Some("summarization"),
        "vision-v1" => Some("vision"),
        _ => None,
    }
}

/// Every workload role the managed backend routes, each with its `hint:*`
/// alias accepted wherever a model id is taken.
pub const WORKLOAD_ROLES: [&str; 7] = [
    "chat",
    "reasoning",
    "agentic",
    "coding",
    "burst",
    "summarization",
    "vision",
];

/// `hint:vision` is deprecated: `vision-v1` silently falls back to the chat
/// default on managed routes, which means every agent still pinned to it
/// loses image/video understanding without any error surfacing (regression
/// R4). Image/video UNDERSTANDING and GENERATION are also separate
/// capabilities that a single `vision` hint cannot distinguish, so each media
/// agent is pinned to its own exact OpenRouter passthrough model instead.
///
/// Qwen3.5 Flash: native tool calling and text+image+video input. Unlike
/// Qwen3.7 Flash, its OpenRouter pricing has no cache-write component, so it
/// passes the backend passthrough catalog's billability filter.
///
/// Used by `vision_agent` (image/video understanding: describe, OCR, chart
/// and UI-element reading).
pub const MODEL_MEDIA_UNDERSTANDING: &str = "openrouter/qwen/qwen3.5-flash-02-23";

/// Same model as [`MODEL_MEDIA_UNDERSTANDING`], pinned separately for
/// `image_agent` (image GENERATION delegate) so the two roles can be retuned
/// independently without one edit silently moving the other.
pub const MODEL_IMAGE_GENERATION_AGENT: &str = "openrouter/qwen/qwen3.5-flash-02-23";

/// Same model as [`MODEL_MEDIA_UNDERSTANDING`], pinned separately for
/// `video_agent` (video GENERATION delegate) so the two roles can be retuned
/// independently without one edit silently moving the other.
pub const MODEL_VIDEO_GENERATION_AGENT: &str = "openrouter/qwen/qwen3.5-flash-02-23";

/// Every managed model id that carries multimodal (image/video) input
/// capability, whether or not it is also the workload's `vision` hint
/// target. `oh_tier_supports_vision` treats membership here the same as the
/// legacy `vision-v1` / `hint:vision` gate, so a media agent pinned to one of
/// these `exact` ids keeps the image/video forwarding path that used to key
/// off the retired hint alone.
pub const MANAGED_MULTIMODAL_MODELS: [&str; 3] = [
    MODEL_MEDIA_UNDERSTANDING,
    MODEL_IMAGE_GENERATION_AGENT,
    MODEL_VIDEO_GENERATION_AGENT,
];
