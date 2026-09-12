//! P-format tool calls — OpenHuman's adapter over
//! [`tinyagents_harness::tool_calling::pformat`].
//!
//! The format itself — the positional `name[arg|arg]` grammar, the schema-driven
//! argument reconstruction, the type coercion, the escape handling — lives in the
//! crate. What stays here is the part that speaks OpenHuman's own tool
//! vocabulary.
//!
//! # Why the crate takes schemas and this takes tools
//!
//! `build_registry` upstream takes `(name, schema)` pairs rather than a tool
//! trait object. A host's tool type is its own vocabulary, and a crate that
//! depended on it could not be used by a second host — which is the whole point
//! of the seam. So the two functions below are the adapter: they read
//! [`Tool::name`] and [`Tool::parameters_schema`] and hand the crate exactly the
//! two things it needs.
//!
//! Everything else is re-exported unchanged, so existing `pformat::…` call sites
//! keep working.

use crate::openhuman::tools::Tool;

pub use tinyagents_harness::tool_calling::{
    parse_call, render_signature, render_signature_from_schema, PFormatParamType, PFormatRegistry,
    PFormatToolParams,
};

/// Build a [`PFormatRegistry`] from the agent's tool slice.
///
/// Call once at construction time, before the tools are moved into the agent —
/// the result is owned and self-contained, so it survives the move without
/// keeping a reference back to the live `Vec<Box<dyn Tool>>` the agent owns.
///
/// The registry is also the safety boundary the format depends on: the parser
/// refuses to invent argument names for a tool it does not know, so a model
/// cannot tunnel arbitrary JSON through by guessing a tool name that does not
/// exist. A registry built from anything other than the agent's real tools
/// would widen that.
pub fn build_registry(tools: &[Box<dyn Tool>]) -> PFormatRegistry {
    build_registry_from_refs(tools.iter().map(|t| t.as_ref()))
}

/// [`build_registry`] over any iterator of borrowed tools.
///
/// A session agent's surface is two sets — the durable registry and the
/// synthesised delegation set (`Agent::synthesized_tools`) — that cannot be
/// merged into one owned slice, since `Box<dyn Tool>` is not cloneable.
pub fn build_registry_from_refs<'a>(
    tools: impl IntoIterator<Item = &'a dyn Tool>,
) -> PFormatRegistry {
    tinyagents_harness::tool_calling::build_registry(
        tools.into_iter().map(|t| (t.name(), t.parameters_schema())),
    )
}

/// Render a single tool's p-format signature, e.g. `get_weather[location|unit]`.
///
/// This signature goes into the tool catalogue in the system prompt, telling the
/// model exactly how to order positional arguments.
pub fn render_signature_from_tool(tool: &dyn Tool) -> String {
    render_signature_from_schema(tool.name(), &tool.parameters_schema())
}

#[cfg(test)]
#[path = "pformat_tests.rs"]
mod tests;
