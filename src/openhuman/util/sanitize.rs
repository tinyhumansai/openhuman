//! Sanitization helpers for untrusted text bound for a model's context.
//!
//! The implementation moved to `tinymcp_bus`, which is where the MCP transports
//! apply it to every remote tool description before a caller sees one. Two
//! copies of a security-relevant stripping rule in two repositories would
//! drift, and this one is also used for *skill* descriptions from the
//! orchestrator prompt builder — so both need the same rule, and one of them
//! has to name the other's.
//!
//! Re-exported under the path this module has always been at, so callers keep
//! their spelling.

pub use tinymcp_bus::sanitize::{
    sanitize_for_llm, strip_control_chars, strip_instruction_fences, truncate_utf8_safe,
    MAX_DESCRIPTION_BYTES, MAX_TITLE_BYTES,
};
