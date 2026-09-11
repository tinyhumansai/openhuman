//! Building the stdin envelope.
//!
//! Every hook event shares a common header — version, workspace roots, session
//! and model identity — and the pieces of it that do not change during a run
//! are resolved once at startup rather than per event. A hook firing on every
//! tool call must not cost a config load.

use std::path::PathBuf;
use std::sync::RwLock;

use super::types::{HookEvent, HookInput, HookPayload};

/// Host facts shared by every hook envelope, resolved once at startup.
#[derive(Debug, Clone, Default)]
pub struct HostContext {
    /// Filesystem roots the agent may act in. First entry is the primary root
    /// and becomes `OPENHUMAN_PROJECT_DIR` for hook processes.
    pub workspace_roots: Vec<PathBuf>,
    /// Core version string.
    pub version: String,
}

static HOST: RwLock<Option<HostContext>> = RwLock::new(None);

/// Install the host facts. Called once during core bootstrap, and again by the
/// RPC reload endpoint when the action dir changes.
pub fn set_host_context(context: HostContext) {
    log::debug!(
        "[hooks] host context: version={} roots={:?}",
        context.version,
        context.workspace_roots
    );
    *HOST.write().expect("hook host context poisoned") = Some(context);
}

/// The installed host facts, or an empty default before bootstrap has run.
pub fn host_context() -> HostContext {
    HOST.read()
        .expect("hook host context poisoned")
        .clone()
        .unwrap_or_default()
}

/// Per-turn identity attached to each envelope.
///
/// All fields are optional because the moments hooks fire at do not all belong
/// to a session — a `sessionStart` has no generation, a CLI turn has no
/// conversation.
#[derive(Debug, Clone, Default)]
pub struct TurnIdentity {
    /// Conversation/thread identifier.
    pub conversation_id: Option<String>,
    /// Per-turn generation identifier.
    pub generation_id: Option<String>,
    /// Agent session identifier.
    pub session_id: Option<String>,
    /// Model driving the turn.
    pub model: Option<String>,
    /// Canonical agent definition id.
    pub agent_id: Option<String>,
    /// Working directory the action is scoped to.
    pub cwd: Option<String>,
}

/// Assemble the envelope handed to a hook on stdin.
pub fn build_input(event: HookEvent, identity: TurnIdentity, payload: HookPayload) -> HookInput {
    let host = host_context();
    HookInput {
        hook_event_name: event.as_str().to_string(),
        conversation_id: identity.conversation_id,
        generation_id: identity.generation_id,
        session_id: identity.session_id,
        model: identity.model,
        agent_id: identity.agent_id,
        openhuman_version: if host.version.is_empty() {
            env!("CARGO_PKG_VERSION").to_string()
        } else {
            host.version
        },
        workspace_roots: host
            .workspace_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect(),
        cwd: identity.cwd,
        payload,
    }
}
