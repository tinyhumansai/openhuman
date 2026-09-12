//! Wiring the pack tools into an agent's registry and visible set.

use std::collections::HashSet;
use std::sync::{Arc, Weak};

use super::registry;
use super::tools::{PackRegistryHandle, UseSkillTool, USE_SKILL};
use crate::openhuman::tools::traits::Tool;

/// Append `use_skill` to a freshly built registry.
///
/// It starts unbound; [`bind_pack_registry`] gives it its view of the registry
/// once that is behind an `Arc`.
pub fn append_pack_tools(tools: &mut Vec<Box<dyn Tool>>) {
    tools.push(Box::new(UseSkillTool::new(PackRegistryHandle::default())));
}

/// Point the pack tool at the durable registry it lives in.
///
/// The handle holds a [`Weak`], so the pack tool referencing the very vector
/// that owns it does not leak. Call this after **every** rebinding of the
/// agent's tool `Arc`; a stale handle degrades to "skill unavailable" rather
/// than dispatching to the wrong registry.
///
/// This is only half the registry. Every `delegate_*` tool lives in the agent's
/// separate `synthesized_tools` `Arc`, so a packed delegate is reachable only
/// once [`bind_synthesized_pack_registry`] has run too.
pub fn bind_pack_registry(tools: &Arc<Vec<Box<dyn Tool>>>) {
    let weak: Weak<Vec<Box<dyn Tool>>> = Arc::downgrade(tools);
    let bound = for_each_pack_tool(tools, |handle| handle.bind(weak.clone()));
    tracing::debug!(bound, "[toolpacks] bound pack tool to durable registry");
}

/// Point the pack tool at the synthesised delegate set.
///
/// `synthesized` is the agent's `synthesized_tools` `Arc`; `durable` is where
/// the pack tool itself lives, since that is the vector to search for it.
///
/// **Call this after every delegation refresh.** `refresh_delegation_tools`
/// replaces the synthesised `Arc` wholesale, and a handle still holding the old
/// `Weak` stops upgrading as soon as the last reader of that allocation goes —
/// at which point every packed delegate reports "no tool in skill" instead of
/// running.
pub fn bind_synthesized_pack_registry(
    durable: &Arc<Vec<Box<dyn Tool>>>,
    synthesized: &Arc<Vec<Box<dyn Tool>>>,
) {
    let weak: Weak<Vec<Box<dyn Tool>>> = Arc::downgrade(synthesized);
    let bound = for_each_pack_tool(durable, |handle| handle.bind_synthesized(weak.clone()));
    tracing::debug!(
        bound,
        delegates = synthesized.len(),
        "[toolpacks] bound pack tool to synthesised delegate set"
    );
}

/// Apply `edit` to every pack tool's handle in `tools`, returning how many.
fn for_each_pack_tool(
    tools: &Arc<Vec<Box<dyn Tool>>>,
    mut edit: impl FnMut(&super::tools::PackRegistryHandle),
) -> usize {
    let mut bound = 0usize;
    for tool in tools.iter() {
        if tool.name() != USE_SKILL {
            continue;
        }
        if let Some(handle) = crate::openhuman::tools::traits::pack_registry_handle(tool.as_ref()) {
            edit(handle);
            bound += 1;
        }
    }
    bound
}

/// Remove packed tool names from an agent's advertised set.
///
/// This is the whole compression: the tools stay registered and executable, but
/// their schemas never reach the provider. An agent that declared none of the
/// packed tools is unaffected, and `use_skill` is only added when the agent
/// actually lost something to a pack — otherwise every narrow sub-agent would
/// grow a tool that can only report an empty skill.
///
/// `agent_id` selects which packs apply: a pack is skipped for the specialist
/// that owns its family (see [`super::types::ToolPack::owners`]), because
/// withholding a belt from the agent that exists to run it only buys a
/// `use_skill` round trip per turn.
///
/// A caller with an *empty* `visible` set means "everything is visible"
/// (the harness's historical sentinel), so there is nothing to subtract from
/// and the set is left alone.
pub fn strip_packed_from_visible(visible: &mut HashSet<String>, agent_id: &str) {
    if visible.is_empty() {
        return;
    }
    // Groups an embedder marked `Advertised` keep their schemas on the wire;
    // `Off` groups were never registered, so nothing of theirs can be in
    // `visible` to subtract. Only `Withheld` — the default for every group —
    // is actually withheld here.
    let groups = super::groups::current();
    let packed: Vec<String> = registry::packed_tool_names_for_agent(agent_id)
        .into_iter()
        .filter(|name| groups.mode_for_tool(name) == super::groups::GroupMode::Withheld)
        .filter(|name| visible.contains(*name))
        .map(str::to_string)
        .collect();
    if packed.is_empty() {
        return;
    }
    for name in &packed {
        visible.remove(name);
    }
    visible.insert(USE_SKILL.to_string());
    tracing::info!(
        agent = %agent_id,
        hidden = packed.len(),
        "[toolpacks] withheld packed tool schemas; use_skill advertised instead"
    );
}
