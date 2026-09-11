//! The tool trait and its vocabulary — the stable host import path.
//!
//! Every definition here lives in [`tinytools`], which `tinyagents` also
//! depends on. That is what makes `tinytools::Tool` and the trait the harness
//! runs a loop over the *same* trait: a tool is implemented once and both sides
//! accept it, with no conversion at the seam to get subtly wrong.
//!
//! ~190 call sites in this crate import from `crate::openhuman::tools::traits`,
//! so this module stays as the path they name rather than rewriting each import
//! to point at the crate. New code may name either.
//!
//! # What did *not* move
//!
//! Everything that decides something. `tinytools` lets a tool declare the
//! privilege it needs and whether it reaches outside the machine; this host
//! decides what to do about those declarations — [`policy`], the security
//! policy, the approval gate and the sandbox are all still ours, and are meant
//! to stay in one auditable place.
//!
//! [`policy`]: crate::openhuman::tools::policy

pub use tinytools::{
    context_detail_from_args, humanize_tool_name, PermissionLevel, Tool, ToolCallOptions,
    ToolCategory, ToolContent, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};

use crate::openhuman::agent::orchestration::tools::DelegationTarget;
use crate::openhuman::agent::tool_policy::GeneratedToolRuntimeContext;
use crate::openhuman::tools::toolpacks::PackRegistryHandle;

/// Reads a tool's pack-registry handle back out of the erased host extension.
///
/// `use_skill` reads the registry it itself lives in, so
/// they cannot be handed it at construction; `toolpacks::bind_pack_registry`
/// finds them in an already-built registry and hands them a `Weak` view of it.
///
/// The handle rides on [`Tool::host_extension`] rather than a typed trait
/// method because it is *this host's* concept — a vocabulary shared with other
/// hosts has no business naming it. Every other tool returns `None` here and
/// pays nothing.
pub fn pack_registry_handle(tool: &dyn Tool) -> Option<&PackRegistryHandle> {
    tool.host_extension()
        .and_then(|any| any.downcast_ref::<PackRegistryHandle>())
}

/// Reads the agent a synthesised `delegate_*` tool routes to.
///
/// The toolpack route hint uses this to answer "which of this pack's owner
/// agents can this session actually reach, and under what tool name?" without
/// keeping a second copy of every agent's `delegate_name`. Asking the session's
/// own tool set is what makes the answer trustworthy: a delegate the session was
/// not built with simply is not there to find, so a hint can never name a call
/// the model cannot make.
///
/// Erased for the same reason as [`pack_registry_handle`].
pub fn delegation_target(tool: &dyn Tool) -> Option<&str> {
    tool.host_extension()
        .and_then(|any| any.downcast_ref::<DelegationTarget>())
        .map(|target| target.0.as_str())
}

/// Reads a tool's generated-tool runtime metadata back out of the erased
/// per-call host extension.
///
/// Generated or externally supplied tools carry this so the agent policy layer
/// can apply provider / capability / risk rules before execution. Built-in
/// tools leave it unset. Erased for the same reason as
/// [`pack_registry_handle`].
pub fn generated_runtime_context(
    tool: &dyn Tool,
    args: &serde_json::Value,
) -> Option<GeneratedToolRuntimeContext> {
    tool.host_call_extension(args)
        .and_then(|any| any.downcast::<GeneratedToolRuntimeContext>().ok())
        .map(|boxed| *boxed)
}

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
