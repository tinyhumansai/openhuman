//! Tool sources: id-keyed factories an outside crate registers so its tools
//! join the agent catalogue.
//!
//! [`all_tools_with_runtime`](super::ops::all_tools_with_runtime) builds the
//! built-in catalogue, splicing every registered source in at its
//! [`ToolSourcePlacement`], then applies the capability filters (`DomainSet`,
//! memory capability, `ToolGroups`). A registered tool is therefore filtered
//! exactly like a built-in one. With nothing registered the catalogue is
//! unchanged.
//!
//! The catalogue order is part of the prompt-cache layout, so a source can ask
//! to sit after a named anchor point instead of at the end. Sources sharing a
//! placement keep registration order.
//!
//! The registry is generic: it knows nothing about what a source builds.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tinytools::Tool;

use crate::config::{BrowserConfig, Config, DelegateAgentConfig, HttpRequestConfig};
use crate::core::slots::HookSlot;
use crate::modules::tinybox::host_runtime::RuntimeAdapter;
use crate::security::SecurityPolicy;

/// What a tool source can see while the catalogue is assembled: the same
/// handles `all_tools_with_runtime` was given.
pub struct ToolSourceCtx<'a> {
    /// The session's configuration snapshot.
    pub config: &'a Arc<Config>,
    /// The root configuration the catalogue is assembled under.
    pub root_config: &'a Config,
    pub security: &'a Arc<SecurityPolicy>,
    pub runtime: &'a Arc<dyn RuntimeAdapter>,
    pub browser_config: &'a BrowserConfig,
    pub http_config: &'a HttpRequestConfig,
    /// The agent's permitted read/write root.
    pub action_dir: &'a Path,
    pub agents: &'a HashMap<String, DelegateAgentConfig>,
    pub approval_workspace_root: Option<&'a Path>,
}

/// A registered tool source.
pub type ToolSourceFactory =
    Arc<dyn for<'a> Fn(&ToolSourceCtx<'a>) -> Vec<Box<dyn Tool>> + Send + Sync>;

/// Anchor point: right after the search tools, before hosting.
pub const ANCHOR_AFTER_SEARCH: &str = "after_search";
/// Anchor point: right after the delegation tool, before the Composio tools.
pub const ANCHOR_AFTER_DELEGATE: &str = "after_delegate";
/// Every anchor `all_tools_with_runtime` splices sources into.
pub const ANCHORS: [&str; 2] = [ANCHOR_AFTER_SEARCH, ANCHOR_AFTER_DELEGATE];

/// Where in the catalogue a source's tools go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolSourcePlacement {
    /// After every built-in tool (the default).
    #[default]
    End,
    /// Right after the named anchor point (one of [`ANCHORS`]). An unknown
    /// anchor falls back to [`ToolSourcePlacement::End`] and logs a warning.
    After(&'static str),
}

#[derive(Clone)]
struct ToolSourceEntry {
    placement: ToolSourcePlacement,
    factory: ToolSourceFactory,
}

static TOOL_SOURCES: HookSlot<ToolSourceEntry> = HookSlot::new("tools::sources");

/// Registers `factory` as the tool source `id` for the whole process, at the
/// end of the catalogue. Re-registering an id replaces its factory and keeps
/// its position.
pub fn register_tool_source<F>(id: impl Into<String>, factory: F)
where
    F: for<'a> Fn(&ToolSourceCtx<'a>) -> Vec<Box<dyn Tool>> + Send + Sync + 'static,
{
    register_tool_source_at(id, ToolSourcePlacement::End, factory);
}

/// Registers a tool source for the whole process at `placement`.
pub fn register_tool_source_at<F>(id: impl Into<String>, placement: ToolSourcePlacement, factory: F)
where
    F: for<'a> Fn(&ToolSourceCtx<'a>) -> Vec<Box<dyn Tool>> + Send + Sync + 'static,
{
    TOOL_SOURCES.register(
        id,
        ToolSourceEntry {
            placement,
            factory: Arc::new(factory),
        },
    );
}

/// Registers a tool source for the context built from `extensions`, at the end
/// of the catalogue.
pub fn register_tool_source_in<F>(
    extensions: &crate::core::slots::Extensions,
    id: impl Into<String>,
    factory: F,
) where
    F: for<'a> Fn(&ToolSourceCtx<'a>) -> Vec<Box<dyn Tool>> + Send + Sync + 'static,
{
    TOOL_SOURCES.register_in(
        extensions,
        id,
        ToolSourceEntry {
            placement: ToolSourcePlacement::End,
            factory: Arc::new(factory),
        },
    );
}

/// Removes the process-wide tool source `id`.
pub fn unregister_tool_source(id: &str) -> bool {
    TOOL_SOURCES.unregister(id)
}

/// The placement of every registered source, in registration order.
pub fn registered_tool_source_placements() -> Vec<(String, ToolSourcePlacement)> {
    TOOL_SOURCES
        .entries()
        .into_iter()
        .map(|(id, entry)| (id, entry.placement))
        .collect()
}

/// Builds the tools of every registered source placed at `anchor`
/// (`Some(name)`) or at the end (`None`, which also collects sources whose
/// anchor is unknown), in registration order.
pub(crate) fn build_registered_tool_sources(
    ctx: &ToolSourceCtx<'_>,
    anchor: Option<&str>,
) -> Vec<Box<dyn Tool>> {
    let mut tools = Vec::new();
    for (id, entry) in TOOL_SOURCES.entries() {
        let applies = match (entry.placement, anchor) {
            (ToolSourcePlacement::After(a), Some(wanted)) => a == wanted,
            (ToolSourcePlacement::After(a), None) if !ANCHORS.contains(&a) => {
                tracing::warn!(source = %id, anchor = a, "[tools::sources] unknown anchor; appending at the end");
                true
            }
            (ToolSourcePlacement::End, None) => true,
            _ => false,
        };
        if !applies {
            continue;
        }
        let built = (entry.factory)(ctx);
        tracing::debug!(
            source = %id,
            anchor = ?anchor,
            count = built.len(),
            "[tools::sources] registered tool source built"
        );
        tools.extend(built);
    }
    tools
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
