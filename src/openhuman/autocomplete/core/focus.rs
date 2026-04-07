//! Accessibility focus, clipboard/paste insertion, and key state probes.
//!
//! Delegates to the shared `accessibility` middleware module.

pub(super) use crate::openhuman::accessibility::apply_text_to_focused_field;
pub(super) use crate::openhuman::accessibility::focused_text_context_verbose;
pub(super) use crate::openhuman::accessibility::is_escape_key_down;
pub(super) use crate::openhuman::accessibility::is_tab_key_down;
pub(super) use crate::openhuman::accessibility::send_backspace;
pub(super) use crate::openhuman::accessibility::validate_focused_target;
