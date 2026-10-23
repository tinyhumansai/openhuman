//! Configurable hooks — user-authored scripts that observe and gate an agent.
//!
//! The engine and `hooks.json` contract are supplied by `tinyagents-hooks`.
//! This module keeps the OpenHuman host lifecycle and its stable RPC surface.

pub mod host;
pub mod ops;
pub mod prompt_eval;
pub mod schemas;
mod spawner;

pub use ops::{init, PromptVerdict};
pub use schemas::{
    all_controller_schemas as all_hooks_controller_schemas,
    all_registered_controllers as all_hooks_registered_controllers,
};
