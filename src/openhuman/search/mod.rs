//! Unified search domain.
//!
//! This is the canonical home for web search selection and agent-facing search
//! tool registration. Search provider implementations live under this module,
//! even when they call shared backend-proxied integration infrastructure.

pub mod registry;
pub mod tools;

pub(crate) mod engines;

pub use registry::build_search_tools;
pub use tools::WebSearchTool;
