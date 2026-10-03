//! JSON-RPC / CLI controller surface for the bundled local AI stack.
//!
//! This module provides high-level functions for interacting with local AI
//! services such as agent chat, summarization, and transcription. The local
//! runtime itself is run by the user; nothing here downloads models. These functions are typically invoked via RPC or CLI.

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

mod agent_chat;
mod runtime_ops;
mod turn_guards;

pub use agent_chat::{
    agent_chat, agent_chat_for, agent_chat_reply_for, agent_chat_simple, AgentChatReply,
    AgentChatTarget,
};
pub use runtime_ops::{
    local_ai_prompt, local_ai_status, local_ai_summarize, local_ai_transcribe,
    local_ai_transcribe_bytes, local_ai_tts, local_ai_vision_prompt,
};

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use turn_guards::{
    effective_agent_chat_origin, grant_turn_cwd, normalize_model_override, resolve_turn_cwd,
};
