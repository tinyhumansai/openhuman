//! Host UI tools: `show_ui`, which renders a skill's own HTML inline in the
//! chat in the sandboxed frame MCP widgets use.
//!
//! Gated on `mcp` with the widget frame it draws into (`mcp::ui`): the
//! document goes through that frame's inline cache and presentation.

pub mod show_ui;

pub use show_ui::ShowUiTool;
