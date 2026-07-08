//! Unified provider abstraction — cloud + local chat, embedding, and streaming.
//!
//! This module was previously `src/openhuman/providers/`. It now lives under
//! `inference/provider/` so all inference concerns (local runtime, cloud
//! providers, HTTP endpoint) share a single domain root.

pub mod auth_error_registry;
pub mod billing_error;
pub mod claude_agent_sdk;
pub mod claude_code;
pub mod compatible;
pub mod compatible_dump;
pub mod compatible_parse;
pub mod compatible_stream;
pub mod compatible_types;
pub mod config_rejection;
pub mod error_classify;
pub mod error_code;
pub mod factory;
mod openai_codex;
pub mod openhuman_backend;
pub mod ops;
pub mod reliable;
pub mod resolved_route;
pub mod router;
pub mod schemas;
pub mod temperature;
pub mod thread_context;
pub mod traits;

#[allow(unused_imports)]
pub use traits::{
    ChatMessage, ChatRequest, ChatResponse, ConversationMessage, PromptCacheCapabilities, Provider,
    ProviderCapabilityError, ProviderDelta, ToolCall, ToolResultMessage, UsageInfo,
    AGENT_TURN_MAX_OUTPUT_TOKENS,
};

pub use billing_error::is_budget_exhausted_message;
pub use config_rejection::{
    is_openai_compatible_unknown_model_message, is_provider_config_rejection_message,
};
pub use error_code::{
    backend_error_code_skips_sentry, body_flags_malformed, extract_backend_error_code,
    extract_backend_error_code_token, is_backend_client_guard_leak,
    is_backend_malformed_bad_request, is_managed_backend_envelope, managed_error_skips_sentry,
    BackendErrorCode,
};
pub(crate) use factory::chat_model_from_provider;
pub(crate) use factory::is_raw_passthrough_model;
pub use factory::{
    create_chat_model, create_chat_model_from_string, create_chat_model_with_model_id,
    create_chat_provider, provider_for_role, role_for_model_tier, BYOK_INCOMPLETE_SENTINEL,
};
pub use ops::*;
pub use resolved_route::{
    current_resolved_provider_route, current_route_slot, record_resolved_provider_route,
    with_resolved_provider_route_scope, with_route_slot, ResolvedProviderRoute, RouteSlot,
};
