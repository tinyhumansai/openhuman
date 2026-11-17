//! Session-scoped rendering and routing for `use_skill` pack disclosure.

use super::*;
use tinyagents_harness_tools::skills::PackRegistryHandle;
use tinytools::ToolResult as TaToolResult;

impl ToolPolicyHostState {
    /// Return the single spawn route when it can dispatch any owner in this
    /// pack. The registered schema carries the actual allowed agent ids.
    pub(super) fn callable_delegates_for(&self, owners: &[&str]) -> Vec<String> {
        let Some(spawn) = self.resolve_tool("spawn_subagent") else {
            return Vec::new();
        };
        if self.session.decision_for("spawn_subagent").is_denied() {
            return Vec::new();
        }
        let parameters = spawn.parameters_schema();
        let allowed = parameters
            .pointer("/properties/agent_id/enum")
            .and_then(serde_json::Value::as_array);
        let can_reach_owner = allowed
            .map(|ids| {
                owners
                    .iter()
                    .any(|owner| ids.iter().any(|id| id.as_str() == Some(owner)))
            })
            .unwrap_or(true);
        if can_reach_owner {
            vec!["spawn_subagent".to_string()]
        } else {
            Vec::new()
        }
    }

    /// The answer to a `use_skill` call naming a tool `pack` does not contain:
    /// the tools in it this session can call, or the pack's route when none.
    pub(super) fn no_such_pack_tool<'a>(
        &self,
        pack: &'static tinyagents_harness_tools::skills::ToolPack,
        tool: &'a str,
    ) -> tinyagents_harness_tools::skills::NoSuchPackTool<'a> {
        let callable = pack
            .tools
            .iter()
            .copied()
            .filter(|name| {
                self.resolve_tool(name).is_some()
                    && !self.session.decision_for(name).blocks_execution()
            })
            .collect();
        tinyagents_harness_tools::skills::NoSuchPackTool {
            not_found_marker: tinytools::status::NOT_FOUND_MARKER,
            skill: pack.id,
            tool,
            callable,
            route: self.route_for_pack(pack),
        }
    }

    /// The route sentence for a pack, resolved against THIS session.
    pub(super) fn route_for_pack(
        &self,
        pack: &tinyagents_harness_tools::skills::ToolPack,
    ) -> String {
        tinyagents_harness_tools::skills::route_sentence(
            &self.callable_delegates_for(pack.owners),
            pack.owners,
        )
    }

    /// Render a `use_skill` listing scoped to what this session may call.
    ///
    /// This lives in the middleware because the middleware is the only layer
    /// that holds the session — `UseSkillTool` is built once per registry and
    /// has no idea who is calling it. Only the disclosure half is rendered here:
    /// a call that names a `tool` is the execution half, which
    /// `channel_permission_block` has already gated and the tool's own
    /// `execute` dispatches. Returns `None` when there is nothing to scope (not
    /// `use_skill`, a tool named, no `skill` argument), so the call falls
    /// through to the tool's own `execute` unchanged.
    pub(super) fn render_skill_for_session(&self, call: &TaToolCall) -> Option<TaToolResult> {
        // Only `use_skill` discloses a pack. Another tool whose arguments carry
        // a `skill` key (an MCP schema, a stray key) must run as itself.
        if call.name != USE_SKILL {
            return None;
        }
        if tinyagents_harness_tools::skills::named_tool(&call.arguments).is_some() {
            return None;
        }
        let skill = call
            .arguments
            .get("skill")
            .and_then(serde_json::Value::as_str)?;
        self.resolve_tool(&call.name)?;
        let durable = self.tool_sets.first()?;
        let handle = PackRegistryHandle::default();
        handle.bind(Arc::downgrade(durable));
        if let Some(synthesized) = self.tool_sets.get(1) {
            handle.bind_synthesized(Arc::downgrade(synthesized));
        }
        let is_callable = |name: &str| !self.session.decision_for(name).blocks_execution();
        let route = crate::tools::toolpacks::pack(skill)
            .map(|pack| self.route_for_pack(pack))
            .unwrap_or_default();
        let rendered = tinyagents_harness_tools::skills::render_pack_filtered(
            &crate::tools::toolpacks::CATALOG,
            skill,
            &handle,
            // The same predicate the gate applies to `use_skill`'s inner tool.
            // Two sources of truth for "can this session call it" is the bug.
            &is_callable,
            &route,
        );
        Some(match rendered {
            Ok(text) => TaToolResult::success(text),
            Err(message) => TaToolResult::error(message),
        })
    }
}
