//! Sanitization helpers for untrusted text bound for a model's context.
//!
//! The implementation is shared from `tinytools_std`; TinyMCP applies it to
//! remote tool descriptions, and OpenHuman also uses it for skill descriptions.
//! Keeping both call sites on the same helper prevents their stripping rules
//! from drifting.
//!
//! Re-exported under the path this module has always been at, so callers keep
//! their spelling.

pub use tinymcp_bus::sanitize::{MAX_DESCRIPTION_BYTES, MAX_TITLE_BYTES};
pub use tinytools_std::sanitize::{
    sanitize_for_llm, strip_control_chars, strip_instruction_fences, truncate_utf8_safe,
};
