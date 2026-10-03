//! Permanent host tool visibility and the managed prompt catalogue.

use super::{OpenHumanTurnPrelude, OpenHumanTurnToolSurface};
use tinyagents_runtime::{PrefixSnapshot, TurnPreparation};

pub(super) fn refresh_visibility(
    surface: &mut OpenHumanTurnToolSurface,
    synthesized: &[Box<dyn tinytools::Tool>],
) {
    crate::tools::toolpacks::strip_packed_from_visible(
        &mut surface.visible_tool_names,
        &surface.agent_definition_name.clone(),
    );
    // Same split as the session host's `recompute_deferred_tool_names`:
    // a `Deferred` synthesised tool leaves the wire and joins the
    // searchable set, on a belt that opted into discovery.
    if surface.discovery_enabled {
        let deferred = crate::tools::implementations::meta::deferred_set(
            surface.tools.as_slice(),
            synthesized,
            &surface.requested_deferred_tools,
        );
        surface
            .visible_tool_names
            .retain(|name| !deferred.contains(name));
        surface.deferred_tool_names = deferred;
    }
    let permanent = surface.permanent_tool_names.clone();
    surface.visible_tool_names.extend(permanent.iter().cloned());
    surface
        .deferred_tool_names
        .retain(|name| !permanent.contains(name));
}

impl OpenHumanTurnPrelude {
    pub(super) fn refresh_permanent_prefix(
        &self,
        preparation: &mut TurnPreparation,
        current: &PrefixSnapshot,
    ) {
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        preparation.prefix = super::super::managed_tools::refresh_prefix(
            preparation.prefix.as_ref().unwrap_or(current),
            &surface.visible_tool_specs,
            &surface.permanent_tool_names,
        )
        .or(preparation.prefix.take());
    }
}
