//! On-demand tool disclosure ("tool packs").
//!
//! A tool's JSON schema is charged on every provider call of every turn,
//! whether or not the tool is used. The Master Agent's belt measured ~21.4k
//! tokens of schema against a ~4.3k-token system prompt, so schemas — not
//! instructions — were the dominant fixed cost, and most of that surface is
//! idle in most conversations.
//!
//! A pack keeps its tools constructed and executable but unadvertised. The
//! agent sees one small tool instead: [`tools::UseSkillTool`] renders a pack's
//! schemas into the conversation when called with a `skill` alone, and executes
//! one of them when also given a `tool`, forwarding permission level and
//! execution context to the real tool so nothing is laundered through the
//! proxy.
//!
//! **Why a proxy and not dynamic registration.** Registering the real schemas
//! mid-turn would be better — the model would get native tool calling with
//! argument validation instead of a nested `args` object. It is not possible
//! today: `tinyagents` registers tools into a plain `HashMap` behind `&mut`
//! before the turn starts (`harness/tool/types.rs`), and the provider-facing
//! schema list is derived once from that. Making the registry interior-mutable
//! and re-deriving schemas per iteration is the upstream change that would
//! retire this proxy.

pub mod groups;
pub mod ops;
pub mod registry;
pub mod tools;
pub mod types;

pub use groups::{GroupMode, ToolGroups, GROUP_COUNT};
pub use ops::{
    append_pack_tools, bind_pack_registry, bind_synthesized_pack_registry,
    strip_packed_from_visible,
};
pub use registry::{
    all_packed_tool_names, callable_pack_ids, pack, pack_for_tool, pack_index_markdown_filtered,
    PACKS,
};
pub use tools::{
    named_tool, render_pack_filtered, route_sentence, scope_use_skill_spec, PackRegistryHandle,
    USE_SKILL,
};
pub use types::ToolPack;

#[cfg(test)]
#[path = "toolpacks_tests.rs"]
mod tests;
