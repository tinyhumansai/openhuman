//! Shared types for the task dispatcher.

use crate::openhuman::todos::ops::BoardLocation;

/// Handle to an in-flight autonomous run, keyed by its session `thread_id`.
///
/// Autonomous runs are detached `tokio` tasks, not web-channel turns, so they
/// are invisible to the web channel's own in-flight registry — which is why the
/// chat **Cancel** button (which calls `channel_web_cancel`) couldn't stop them.
/// Registering the run's [`AbortHandle`](tokio::task::AbortHandle) here lets
/// [`cancel_session`] abort it from that same cancel path.
pub(super) struct ActiveRun {
    pub(super) abort: tokio::task::AbortHandle,
    pub(super) hb_cancel: tokio::sync::watch::Sender<bool>,
    pub(super) location: BoardLocation,
    pub(super) card_id: String,
    pub(super) run_id: String,
}

/// A resolved executor: which built-in agent definition to build, an optional
/// system-prompt suffix carrying a personality identity or skill guidelines,
/// the resolved agent profile (when the handle is a personality) so its
/// per-profile allowlists are enforced, and a label for logs/telemetry.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedExecutor {
    pub(super) agent_id: String,
    pub(super) prompt_suffix: Option<String>,
    /// The resolved profile for personality executors. `None` for skill /
    /// built-in / default executors → unrestricted (legacy behaviour). When
    /// present, the autonomous run applies the profile's tool/skill/MCP/
    /// connector and memory-source gates, not just its SOUL/MEMORY text.
    pub(super) profile: Option<crate::openhuman::profiles::AgentProfile>,
    pub(super) label: String,
}

impl ResolvedExecutor {
    pub(super) fn default_agent() -> Self {
        Self {
            agent_id: "orchestrator".to_string(),
            prompt_suffix: None,
            profile: None,
            label: "default".to_string(),
        }
    }
}

/// Outcome of a dispatch attempt.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// The card was claimed and a detached autonomous run was spawned.
    Running { run_id: String },
    /// Plan approval is required; the card was parked at `awaiting_approval`
    /// and a `TaskPlanAwaitingApproval` event was emitted. No run was spawned.
    AwaitingApproval,
}
