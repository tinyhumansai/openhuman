//! Tool groups as a **library-level axis**, alongside `ServiceSet` and
//! `DomainSet`.
//!
//! The packs in [`super::registry`] exist for one host's problem: an
//! orchestrator whose fixed per-turn cost is dominated by tool schemas. That is
//! a compression decision, and compiling it in is right for the desktop app —
//! membership must not be editable by config or RPC, or a caller could move a
//! dangerous tool out of the reviewed surface.
//!
//! An embedder is a different question. `openhuman_core` is consumed as a
//! library through [`Harness`](crate::Harness), and there the pack table is not
//! a compression choice but a *capability* one: a host embedding the harness to
//! summarise documents has no use for the crypto belt at any disclosure level,
//! and a host driving its own routing may want every schema on the wire because
//! it does not pay the orchestrator's budget. Neither is expressible by
//! membership alone, which only ever answers "advertised or withheld".
//!
//! So the group id — the same string the model names in `use_skill` — becomes
//! the unit an embedder selects on, with three states rather than two:
//!
//! | [`GroupMode`] | Schemas on the wire | Registered and callable |
//! | --- | --- | --- |
//! | `Advertised` | yes | yes |
//! | `Withheld` | no (reached via `use_skill`) | yes |
//! | `Off` | no | **no** |
//!
//! `Off` is the state that could not be said before, and it is the one an
//! embedder reaches for most: absence beats a registered tool that fails, for
//! the reason the `flows` compile gate already documents — a tool the model can
//! see teaches it the capability exists and makes it retry.
//!
//! **The default is exactly today's behaviour.** [`ToolGroups::default`] puts
//! every pack in `Withheld`, which is what the compiled-in table meant before
//! this type existed, so a host that never calls
//! [`CoreBuilder::tool_groups`](crate::core::runtime::CoreBuilder::tool_groups)
//! is unaffected.
//!
//! **This axis does not widen what a build contains.** A group whose tools are
//! compiled out (`--no-default-features`) or whose `DomainGroup` is off under
//! the ambient `DomainSet` stays absent no matter what mode is set here;
//! `Advertised` cannot conjure a tool that was never registered. The three
//! filters compose one way only — narrowing.

use super::registry::PACKS;

/// How one tool group reaches the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupMode {
    /// Schemas are on the wire on every provider call.
    Advertised,
    /// Registered and executable, but reached only through `use_skill`.
    /// The compiled-in default for every pack.
    #[default]
    Withheld,
    /// Not registered at all — the tools do not exist for this core.
    Off,
}

/// The number of compiled-in groups. `const` so [`ToolGroups`] can be a fixed
/// array and carry no allocation.
pub const GROUP_COUNT: usize = PACKS.len();

/// Per-group disclosure for one core.
///
/// Indexed positionally against [`PACKS`]; ids are resolved through
/// [`ToolGroups::index_of`] so a caller never depends on pack order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolGroups {
    modes: [GroupMode; GROUP_COUNT],
}

impl Default for ToolGroups {
    /// Every group withheld — byte-identical to the behaviour before this type
    /// existed.
    fn default() -> Self {
        Self {
            modes: [GroupMode::Withheld; GROUP_COUNT],
        }
    }
}

impl ToolGroups {
    /// Every group withheld. The desktop app's shape, and the default.
    pub fn packed() -> Self {
        Self::default()
    }

    /// Every group's schemas on the wire.
    ///
    /// For a host that does not pay the orchestrator's per-turn schema budget —
    /// a short-lived harness run, or an embedder doing its own routing — and
    /// wants native function calling rather than the `use_skill` envelope.
    pub fn advertised() -> Self {
        Self {
            modes: [GroupMode::Advertised; GROUP_COUNT],
        }
    }

    /// No group's tools registered at all: the baseline belt only.
    ///
    /// The starting point for a host that opts individual groups back in, the
    /// way `DomainSet::kernel()` is for domains.
    pub fn none() -> Self {
        Self {
            modes: [GroupMode::Off; GROUP_COUNT],
        }
    }

    /// Set one group's mode. Unknown ids are ignored — a group id is data, and
    /// a build that compiled a family out should not panic a host that still
    /// names it.
    pub fn with(mut self, id: &str, mode: GroupMode) -> Self {
        if let Some(i) = Self::index_of(id) {
            self.modes[i] = mode;
        } else {
            log::warn!(
                "[toolgroups] ignoring unknown group id `{id}` (not compiled into this build)"
            );
        }
        self
    }

    /// The mode for `id`. An unknown id reports [`GroupMode::Advertised`],
    /// because a tool that belongs to no compiled-in group is never withheld.
    pub fn mode(&self, id: &str) -> GroupMode {
        Self::index_of(id)
            .map(|i| self.modes[i])
            .unwrap_or(GroupMode::Advertised)
    }

    /// The mode owning `tool`, or [`GroupMode::Advertised`] when no group does.
    pub fn mode_for_tool(&self, tool: &str) -> GroupMode {
        match super::registry::pack_for_tool(tool) {
            Some(pack) => self.mode(pack.id),
            None => GroupMode::Advertised,
        }
    }

    /// Every compiled-in group id, in table order.
    pub fn ids() -> impl Iterator<Item = &'static str> {
        PACKS.iter().map(|p| p.id)
    }

    fn index_of(id: &str) -> Option<usize> {
        PACKS.iter().position(|p| p.id == id)
    }
}

/// The ambient groups for the running core, or the default when there is no
/// [`CoreContext`](crate::core::runtime::context::CoreContext) — unit tests and
/// pre-boot CLI paths, which must behave as they did before.
pub fn current() -> ToolGroups {
    crate::core::runtime::context::CoreContext::current()
        .map(|c| c.tool_groups())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "groups_tests.rs"]
mod tests;
