//! Free `render_*` functions, sub-agent prompt renderer, and workspace-file
//! I/O helpers.
//!
//! The `render_*` family provides a functional interface over the section
//! structs in [`super::sections`] — `agents/<id>/prompt.rs` builders call
//! these to assemble their own final system prompt without needing the full
//! [`super::builder::SystemPromptBuilder`] machinery.

include!("render_helpers_part_01.rs");
include!("render_helpers_part_02.rs");
