//! Terminal app detection and context extraction.
//!
//! Delegates to the shared `accessibility` middleware module.

pub(super) use crate::openhuman::accessibility::extract_terminal_input_context;
pub(super) use crate::openhuman::accessibility::is_terminal_app;
pub(super) use crate::openhuman::accessibility::looks_like_terminal_buffer;
