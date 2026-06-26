//! Voice-driven desktop actions domain.
//!
//! Maps recognized utterances to controller-backed actions with safety levels,
//! confirmation flows, and execution tracking.
//!
//! Log prefix: `[voice_actions]`

pub mod engine;
pub mod llm_intent;
mod rpc;
mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_voice_actions_controller_schemas,
    all_registered_controllers as all_voice_actions_registered_controllers,
    schemas as voice_actions_schemas,
};
pub use types::{ActionSafety, IntentStatus, VoiceIntent};
