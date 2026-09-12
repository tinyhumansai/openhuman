//! Tool-pack types: the unit of on-demand tool disclosure.

/// A named bundle of tools that is **not** advertised to the model by default.
///
/// The pack's tools stay fully constructed and executable; what changes is that
/// their JSON schemas never reach the provider until the agent asks for them.
/// That trade is the whole point: an orchestrator carrying ~77 tool schemas
/// spends far more of its fixed per-turn budget on schemas than on its own
/// instructions, and most of those tools go untouched in most conversations.
pub struct ToolPack {
    /// Stable id the agent names in `use_skill`.
    pub id: &'static str,
    /// One line, rendered in the always-on pack index. This is the only text
    /// about the pack the model sees before loading it, so it has to carry
    /// enough intent for the model to know when to reach for it.
    pub summary: &'static str,
    /// Tool names this pack owns. A name listed here is removed from the
    /// agent's advertised surface and reachable only through `use_skill`.
    pub tools: &'static [&'static str],
    /// Agent ids for which this pack is **not** applied.
    ///
    /// Withholding is a bet that the tools are idle in most turns. That bet is
    /// wrong for the specialist a family was delegated to: `settings_agent`
    /// exists precisely to run `config_*` / `health_*` / `service_*`, so
    /// packing them would put a `use_skill` round trip in front of the first
    /// call of every one of its turns and buy nothing — its whole belt is the
    /// pack.
    ///
    /// The earlier packs did not need this because they held only synthesised
    /// `delegate_*` tools, which exist on the orchestrator alone. Packs over
    /// raw tools do, and an owner list is the narrowest way to say so.
    pub owners: &'static [&'static str],
}

impl ToolPack {
    pub fn owns(&self, tool: &str) -> bool {
        self.tools.contains(&tool)
    }

    /// Whether `agent_id` is the specialist this pack's family belongs to.
    pub fn is_owner(&self, agent_id: &str) -> bool {
        self.owners.contains(&agent_id)
    }
}
