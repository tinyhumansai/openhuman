//! On-demand tool disclosure ("tool packs").
//!
//! A tool's JSON schema is charged on every provider call of every turn,
//! whether or not the tool is used. The Master Agent's belt measured ~21.4k
//! tokens of schema against a ~4.3k-token system prompt, so schemas — not
//! instructions — were the dominant fixed cost, and most of that surface is
//! idle in most conversations.
//!
//! A pack keeps its tools constructed and executable but unadvertised. The
//! agent sees one small tool instead: `tinyagents_harness_tools::skills::UseSkillTool` renders a pack's
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

pub use groups::{set_process_default, GroupMode, ToolGroups, GROUP_COUNT};
pub(crate) use ops::is_canonical_use_skill_tool;
pub use ops::{
    append_pack_tools, bind_pack_registry, bind_synthesized_pack_registry, close_handed_off_packs,
    closed_by_direct_handoff, strip_packed_from_visible,
};
pub use registry::{
    all_packed_tool_names, callable_pack_ids, pack, pack_for_tool, pack_index_markdown_filtered,
    CATALOG, PACKS,
};

#[cfg(test)]
#[path = "toolpacks_tests.rs"]
mod tests;
