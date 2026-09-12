//! Agent turn workspace — the per-turn filesystem root an embedder binds a
//! single agent turn to.
//!
//! A host that drives the core in-process (the Medulla TUI's embedded core, a
//! workflow node dispatched to the `openhuman` harness) runs one turn *against
//! a checkout it names*, not against the operator's ambient
//! [`Config::action_dir`](crate::openhuman::config::Config). Everything the
//! core already had for this was **process-global**: `action_dir` comes from
//! config, and [`SecurityPolicy`](crate::openhuman::security::SecurityPolicy)
//! is installed once per process, so two concurrent turns could not disagree
//! about where they are allowed to write.
//!
//! This module is the per-turn answer, deliberately shaped like
//! [`super::turn_origin`]: a [`tokio::task_local`] the entry point scopes
//! around its `run_turn` invocation, read by the two places that decide where
//! a turn may work:
//!
//! * the session builder
//!   ([`Agent::from_config`](crate::openhuman::agent::Agent::from_config)),
//!   which turns it into the turn's
//!   [`WorkspaceDescriptor`](tinyagents_harness::workspace::WorkspaceDescriptor)
//!   so acting tools (shell, file, git) resolve their default cwd there; and
//! * [`SecurityPolicy::is_within_trusted_root`](crate::openhuman::security::SecurityPolicy::is_within_trusted_root),
//!   which treats the scoped root as a read/write trusted root for the
//!   duration of the turn, so a write into that checkout is not refused as an
//!   escape from `workspace_dir`.
//!
//! # Trust
//!
//! Scoping a root here is a **grant**, exactly as strong as a user-configured
//! [`TrustedRoot`](crate::openhuman::security::TrustedRoot) with
//! `access = ReadWrite`, and it is scoped to one turn rather than persisted.
//! It therefore never weakens the two invariants a trusted root has never been
//! able to weaken: credential stores
//! ([`SecurityPolicy::is_always_forbidden`](crate::openhuman::security::SecurityPolicy::is_always_forbidden))
//! and workspace-internal application state stay unreachable. An embedder
//! scopes it only for a root it already decided the turn owns — a workflow
//! run's checkout — which is why the API takes an owned [`PathBuf`] the caller
//! resolved rather than a string the model could have supplied.
//!
//! Nothing is scoped by default: `current()` returning `None` is every
//! existing caller, and leaves path policy byte-identical.

use std::path::PathBuf;

tokio::task_local! {
    /// Per-turn filesystem root. Scoped by an embedder around the agent turn it
    /// drives; read by the session builder and the path policy. Unset for every
    /// caller that has not opted in.
    pub static AGENT_TURN_WORKSPACE: PathBuf;
}

/// Scope `root` as the workspace for the duration of `fut`.
///
/// `root` should be an existing, absolute directory the caller has already
/// resolved — a relative or missing path still scopes, but the tools rooted at
/// it will fail on their own terms rather than being caught here, and the
/// policy grant only ever covers paths that actually resolve under it.
///
/// The inner future is `Box::pin`-ed before entering the task-local scope for
/// the same reason [`super::turn_origin::with_origin`] does it: the agent loop
/// downstream is deep, and stacking task-local scopes plus the loop on a 2 MiB
/// worker stack blows the test runtime.
pub async fn with_workspace<F: std::future::Future>(root: PathBuf, fut: F) -> F::Output {
    AGENT_TURN_WORKSPACE.scope(root, Box::pin(fut)).await
}

/// The workspace root scoped for the current turn, when an embedder set one.
///
/// `None` — the default everywhere — means "no per-turn root": the session
/// binds no [`WorkspaceDescriptor`](tinyagents_harness::workspace::WorkspaceDescriptor)
/// of its own and the path policy grants nothing beyond its configured roots.
pub fn current() -> Option<PathBuf> {
    AGENT_TURN_WORKSPACE.try_with(|root| root.clone()).ok()
}

/// Carry the workspace scoped **right now** into a future that will run on
/// another task.
///
/// Same contract, and same reason, as
/// [`turn_origin::propagate`](super::turn_origin::propagate): a detached
/// sub-agent starts on a fresh task where [`current`] is `None`, so it would
/// lose the root its parent turn was bound to and have its writes into that
/// checkout refused. Build the future on the parent task, before `tokio::spawn`
/// — the root is read when this is called, not when the future is polled.
///
/// With no scoped root this is a no-op, which is every caller that never opted
/// in.
pub fn propagate<F: std::future::Future>(fut: F) -> impl std::future::Future<Output = F::Output> {
    let captured = current();
    async move {
        match captured {
            Some(root) => with_workspace(root, fut).await,
            None => fut.await,
        }
    }
}

#[cfg(test)]
#[path = "turn_workspace_tests.rs"]
mod tests;
