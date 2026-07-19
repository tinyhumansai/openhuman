//! Storage layer for tool-scoped rules — thin host shim over
//! `tinycortex::memory::tool_memory::store` (W7).
//!
//! The store engine (put / get / list / delete / prompt over an
//! `Arc<dyn Memory>`) and the `Memory` trait are both owned by tinycortex.

use std::sync::Arc;

use crate::openhuman::memory::Memory;

pub use tinycortex::memory::tool_memory::store::{ToolMemoryStore, TOOL_MEMORY_PROMPT_CAP};

/// Build a crate [`ToolMemoryStore`] over the shared tinycortex memory trait.
pub fn tool_memory_store(memory: Arc<dyn Memory>) -> ToolMemoryStore {
    ToolMemoryStore::new(memory)
}
