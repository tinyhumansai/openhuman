//! Wiring the pack tools into an agent's registry and visible set.

use std::collections::HashSet;
use std::sync::{Arc, Weak};

use super::registry;
use tinyagents_harness_tools::skills::{PackRegistryHandle, UseSkillTool, USE_SKILL};
use tinytools::Tool;

/// Append `use_skill` to a freshly built registry and return the handle its
/// owner must retain for later rebinding.
pub fn append_pack_tools(tools: &mut Vec<Box<dyn Tool>>) -> PackRegistryHandle {
    let handle = PackRegistryHandle::default();
    tools.push(Box::new(UseSkillTool::new(
        handle.clone(),
        registry::CATALOG,
    )));
    handle
}

/// Whether `tool` is the canonical OpenHuman `use_skill` proxy built from the
/// compiled pack catalog. Public callers of `all_tools()` cannot carry the
/// handle separately, so the session builder uses this signature to replace
/// only that proxy with a session-owned handle before binding it.
pub(crate) fn is_canonical_use_skill_tool(tool: &dyn Tool) -> bool {
    if tool.name() != USE_SKILL
        || tool
            .host_extension()
            .and_then(|extension| {
                extension.downcast_ref::<tinyagents_harness::tool::ToolCallBinding>()
            })
            .is_none()
    {
        return false;
    }
    let canonical = UseSkillTool::new(PackRegistryHandle::default(), registry::CATALOG);
    tool.description() == canonical.description()
        && tool.parameters_schema() == canonical.parameters_schema()
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
pub fn bind_pack_registry(handle: &PackRegistryHandle, tools: &Arc<Vec<Box<dyn Tool>>>) {
    let weak: Weak<Vec<Box<dyn Tool>>> = Arc::downgrade(tools);
    handle.bind(weak);
    tracing::debug!(
        tools = tools.len(),
        "[toolpacks] bound pack tool to durable registry"
    );
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
    handle: &PackRegistryHandle,
    synthesized: &Arc<Vec<Box<dyn Tool>>>,
) {
    let weak: Weak<Vec<Box<dyn Tool>>> = Arc::downgrade(synthesized);
    handle.bind_synthesized(weak);
    tracing::debug!(
        delegates = synthesized.len(),
        "[toolpacks] bound pack tool to synthesised delegate set"
    );
}

/// Apply `edit` to every pack tool's handle in `tools`, returning how many.
fn for_each_pack_tool(
    tools: &Arc<Vec<Box<dyn Tool>>>,
    mut edit: impl FnMut(&PackRegistryHandle),
) -> usize {
    let mut bound = 0usize;
    for tool in tools.iter() {
        if tool.name() != USE_SKILL {
            continue;
        }
        if let Some(handle) = crate::tools::host_extensions::pack_registry_handle(tool.as_ref()) {
            edit(&handle);
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

/// Is `tool` withheld from `agent_id` by the pack table right now?
///
/// The predicate behind [`strip_packed_from_visible`], exposed for callers that
/// build a listing of tools rather than a visible set. This keeps packed tools
/// out of listings while preserving their registered dispatch paths.
pub fn is_withheld_from(agent_id: &str, tool: &str) -> bool {
    let groups = super::groups::current();
    groups.mode_for_tool(tool) == super::groups::GroupMode::Withheld
        && registry::packed_tool_names_for_agent(agent_id)
            .into_iter()
            .any(|name| name == tool)
}

/// Pack tools `agent_id` must not reach through `use_skill`, because it can hand
/// the whole family to the pack's owner directly (#6302).
///
/// A pack's raw tools are the owning specialist's belt. When the caller also
/// holds a direct hand-off to that specialist, offering the same raw tools one
/// `use_skill` away gives the model two routes to the same work, and a live
/// account showed which one it takes: it searched and installed skills itself,
/// guessed at tool names, and never handed off. Closing the raw tools leaves the
/// hand-off as the route, and the pack listing then names it (`route_sentence`).
///
/// Close packs whose task family this agent can reach through its
/// `spawn_subagent` schema. The schema's agent-id enum is the same allowlist
/// enforced during execution. The host's explicit closure-owner table is used
/// instead of every `ToolPack::owners` entry because some agents own only
/// auxiliary tools in a broader pack.
pub fn closed_by_direct_handoff(agent_id: &str, tools: &[&dyn Tool]) -> Vec<&'static str> {
    let reachable_owners: HashSet<String> = tools
        .iter()
        .filter(|tool| tool.name() == "spawn_subagent")
        .flat_map(|tool| {
            let schema = tool.parameters_schema();
            schema
                .pointer("/properties/agent_id/enum")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    if reachable_owners.is_empty() {
        return Vec::new();
    }
    let groups = super::groups::current();
    registry::PACKS
        .iter()
        .filter(|pack| !pack.is_owner(agent_id))
        .filter(|pack| {
            registry::DIRECT_HANDOFF_PACK_OWNERS
                .iter()
                .find(|(pack_id, _)| *pack_id == pack.id)
                .is_some_and(|(_, owners)| {
                    owners.iter().any(|owner| reachable_owners.contains(*owner))
                })
        })
        .flat_map(|pack| pack.tools.iter().copied())
        .filter(|name| groups.mode_for_tool(name) == super::groups::GroupMode::Withheld)
        .collect()
}

/// Apply [`closed_by_direct_handoff`] to a freshly built policy session: every
/// closed tool becomes `Deny`, so the `use_skill` gate, the pack listing and the
/// bare-call router all refuse it from one decision.
pub fn close_handed_off_packs(
    session: &mut tinytools::ToolPolicySession,
    agent_id: &str,
    tools: &[&dyn Tool],
) {
    let closed = closed_by_direct_handoff(agent_id, tools);
    if closed.is_empty() {
        return;
    }
    session.deny(closed.iter().copied());
    tracing::debug!(
        agent = %agent_id,
        closed = closed.len(),
        "[toolpacks] closed pack tools whose owner is one direct hand-off away"
    );
}
