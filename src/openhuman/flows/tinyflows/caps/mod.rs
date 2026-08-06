//! OpenHuman adapters for the `tinyflows::caps` host seam.
//!
//! This module stays export-focused. Capability construction, curation,
//! preflight, and invocation logic live in [`ops`]; individual trait adapters
//! live in their focused sibling modules.

mod agent;
mod code;
mod http;
mod llm;
mod mocks;
mod ops;
mod prompt;
mod resolver;
mod state;
mod tier;
pub(crate) mod tools;

// Preserve the existing `caps::X` paths used by flows and adapter siblings.
pub(crate) use agent::*;
pub(crate) use code::*;
pub(crate) use http::*;
pub(crate) use llm::*;
pub(crate) use mocks::*;
pub use ops::*;
pub(crate) use prompt::*;
pub(crate) use resolver::*;
pub(crate) use state::*;
pub(crate) use tier::*;
pub(crate) use tools::NATIVE_TOOL_PREFIX;
