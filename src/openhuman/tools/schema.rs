//! JSON Schema cleaning and validation for LLM tool-calling compatibility.
//!
//! The generic implementation lives in TinyAgents. OpenHuman keeps this module
//! as the stable host import path for existing tools and controller code.

pub use tinyagents::harness::tool::{CleaningStrategy, SchemaCleanr, GEMINI_UNSUPPORTED_KEYWORDS};
