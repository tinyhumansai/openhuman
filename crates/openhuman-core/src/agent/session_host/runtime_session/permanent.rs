//! Permanent host tool visibility and the managed prompt catalogue.

use super::OpenHumanTurnPrelude;
use tinyagents_runtime::{PrefixSnapshot, TurnPreparation};

/// Opens the system message that carries the permanent-tool catalogue.
pub(super) const PERMANENT_TOOLS_MARKER: &str = "<openhuman-permanent-tools>\n";

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
        preparation.prefix = tinyagents_runtime::refresh_prefix(
            preparation.prefix.as_ref().unwrap_or(current),
            &surface.visible_tool_specs,
            &surface.permanent_tool_names,
            PERMANENT_TOOLS_MARKER,
            "</openhuman-permanent-tools>",
        )
        .or(preparation.prefix.take());
    }
}

#[cfg(test)]
#[path = "permanent_tests.rs"]
mod tests;
