//! Host layer over [`tinymemory_core::tool_memory`].
//!
//! The rule store and its types are core. What stays here is the pair that
//! plugs into the agent harness — the post-turn capture hook and the system
//! prompt section — both of which name harness traits (`PostTurnHook`,
//! `PromptContext`) that the engine crate cannot see.

pub use tinymemory_core::tool_memory::*;

pub mod capture;
pub mod prompt;
