//! Domain types for the tool-scoped memory layer — thin host re-export of
//! `tinycortex::memory::tool_memory::types` (W7).
//!
//! [`ToolMemoryRule`] / [`ToolMemoryPriority`] / [`ToolMemorySource`] and the
//! [`tool_memory_namespace`] helper are the crate's (a byte-identical port,
//! preserving the serde wire strings + `rule/{id}` storage keys). Host consumers
//! keep their `memory_tools::types::*` import paths unchanged.

pub use tinycortex::memory::tool_memory::types::{
    tool_memory_namespace, ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
};
