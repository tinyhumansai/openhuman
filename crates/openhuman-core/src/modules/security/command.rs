//! Typed host operations for TinySecurity command policy.

use std::path::Path;

use tinysecurity_bus::{CommandClass, CommandDecision, CommandPolicy};

pub(crate) use super::gap::CommandOp;

/// Classify or admit a command using the module-owned command policy.
pub(crate) async fn command_decision(
    workspace_dir: &Path,
    policy: CommandPolicy,
    command: &str,
    declared: Option<CommandClass>,
    op: CommandOp,
) -> Result<CommandDecision, String> {
    super::gap::command_decision(
        workspace_dir.to_path_buf(),
        policy,
        command.to_owned(),
        declared,
        op,
    )
    .await
}

/// Blocking class-only query is unavailable until an async admission has
/// populated the host's decision snapshot; callers must treat it as a refusal.
pub(crate) fn command_decision_blocking(
    _workspace_dir: &Path,
    _policy: CommandPolicy,
    _command: &str,
    _declared: Option<CommandClass>,
    _op: CommandOp,
) -> Result<CommandDecision, String> {
    Err("TinySecurity command classification requires async admission".into())
}
