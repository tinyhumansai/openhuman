//! OpenHuman-owned metadata recovered from TinyTools' erased extension seams.
//!
//! These helpers are intentionally separate from the `tinytools` vocabulary:
//! pack registries, delegation targets, and generated-tool runtime metadata are
//! host implementation details, not portable tool-trait APIs.

use crate::agent::orchestration::tools::DelegationTarget;
use crate::agent::tool_policy::GeneratedToolRuntimeContext;
use crate::tools::toolpacks::PackRegistryHandle;
use tinytools::Tool;
#[cfg(test)]
use tinytools::{PermissionLevel, ToolCategory, ToolResult, ToolScope};

/// Reads a tool's pack-registry handle from its erased host extension.
pub fn pack_registry_handle(tool: &dyn Tool) -> Option<&PackRegistryHandle> {
    tool.host_extension()
        .and_then(|any| any.downcast_ref::<PackRegistryHandle>())
}

/// Reads the target agent a synthesized `delegate_*` tool routes to.
pub fn delegation_target(tool: &dyn Tool) -> Option<&str> {
    tool.host_extension()
        .and_then(|any| any.downcast_ref::<DelegationTarget>())
        .map(|target| target.0.as_str())
}

/// Reads generated-tool runtime metadata from the erased per-call extension.
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
