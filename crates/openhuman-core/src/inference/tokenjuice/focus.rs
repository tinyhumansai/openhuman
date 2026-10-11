//! `summary_focus`: the argument a tool opts into so its caller can steer the
//! summary of a large result.
//!
//! A tool opts in by adding [`summary_focus_property`] to its schema's
//! `properties`. `ToolOutputMiddleware` takes the value out of the call in
//! `before_tool` — before schema validation and before the tool runs, so the
//! tool body never sees it — and hands it to TinyJuice with the result.

use serde_json::Value;

pub use tinyjuice_bus::tools::{summary_focus_property, SUMMARY_FOCUS_ARG};

/// Whether a tool's parameter schema declares TinyJuice's `summary_focus`,
/// as opposed to a parameter of its own that happens to share the name.
pub fn declares_summary_focus(parameters: &Value) -> bool {
    parameters
        .get("properties")
        .and_then(|properties| properties.get(SUMMARY_FOCUS_ARG))
        == Some(&summary_focus_property())
}

/// Remove `summary_focus` from a call's arguments, returning it when it holds
/// non-blank text.
pub fn take_summary_focus(arguments: &mut Value) -> Option<String> {
    let removed = arguments.as_object_mut()?.remove(SUMMARY_FOCUS_ARG)?;
    removed
        .as_str()
        .map(str::trim)
        .filter(|focus| !focus.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
