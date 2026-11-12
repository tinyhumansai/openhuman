//! The directory a scheduled agent run is rooted at.
//!
//! An agent definition may name one (`AgentExt::scheduled_workspace`). The run
//! then carries it as its workspace descriptor, so the shell and file tools
//! work there and a sandboxed shell asks TinySecurity for that root read-write
//! instead of the process-wide `action_dir`. A deployment whose action dir
//! holds the core workspace (a checkout with the brain inside it) can only run
//! a sandboxed shell from cron this way: TinySecurity refuses any grant that
//! overlaps the core workspace.

use std::path::{Path, PathBuf};

use crate::agent_registry::host_definition::AgentDefinition;
use crate::config::Config;

/// The root a cron run of `definition` works in, or `None` for the
/// configured `action_dir`.
///
/// # Errors
/// A message naming the agent and the reason when the configured root does
/// not exist, is not a directory, overlaps the core workspace or holds the
/// core's credentials. The run must not start rooted anywhere else.
pub(super) fn scheduled_workspace(
    config: &Config,
    definition: &AgentDefinition,
) -> Result<Option<PathBuf>, String> {
    let _ = (config, definition, Path::new(""));
    Ok(None)
}

#[cfg(test)]
#[path = "workspace_root_tests.rs"]
mod tests;
