//! The `reasoning_agent` built-in: the deep-thinking reasoning core (`execute`
//! node) of the orchestration wake graph. Registered in the built-in loader
//! ([`crate::openhuman::agent_registry::agents::loader`]).
//!
//! The current subconscious steering directive for a cycle is carried into the
//! agent's system prompt via a task-local ([`steering::ORCHESTRATION_STEERING`])
//! that the `execute` node scopes around the turn (see [`with_steering`]);
//! `prompt::build` reads it (or falls back to [`DEFAULT_STEERING`]).

pub mod graph;
pub mod prompt;
pub mod steering;

pub use steering::{current_steering, with_steering, DEFAULT_STEERING};
