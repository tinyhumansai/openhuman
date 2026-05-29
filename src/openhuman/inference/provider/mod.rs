//! Unified provider abstraction — cloud + local chat, embedding, and streaming.
//!
//! This module was previously `src/openhuman/providers/`. It now lives under
//! `inference/provider/` so all inference concerns (local runtime, cloud
//! providers, HTTP endpoint) share a single domain root.

pub mod billing_error;
pub mod claude_agent_sdk;
pub mod compatible;
pub mod compatible_dump;
pub mod compatible_parse;
pub mod compatible_stream;
pub mod compatible_types;
pub mod config_rejection;
pub mod factory;
mod openai_codex;
pub mod openhuman_backend;
pub mod ops;
pub mod reliable;
pub mod router;
pub mod schemas;
pub mod temperature;
pub mod thread_context;
pub mod traits;

#[allow(unused_imports)]
pub use traits::{
    ChatMessage, ChatRequest, ChatResponse, ConversationMessage, Provider, ProviderCapabilityError,
    ProviderDelta, ToolCall, ToolResultMessage, UsageInfo,
};

pub use billing_error::is_budget_exhausted_message;
pub use config_rejection::{
    is_openai_compatible_unknown_model_message, is_provider_config_rejection_message,
};
pub use factory::{create_chat_provider, provider_for_role, BYOK_INCOMPLETE_SENTINEL};
pub use ops::*;
