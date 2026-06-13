//! Types for the active-run queue model.

use std::fmt;

/// How a message arriving during an active agent turn should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    /// Abort the in-flight turn and start fresh (default, backward-compatible).
    Interrupt,
    /// Inject the message at the next safe iteration boundary so the agent
    /// sees it mid-turn without restarting.
    Steer,
    /// Queue the message as a follow-up turn that fires after the current
    /// turn completes.
    Followup,
    /// Silently collect the message as additional context; the agent sees it
    /// at the next iteration boundary but does not treat it as a new instruction.
    Collect,
    /// Run as an independent concurrent turn on the same thread. The new turn
    /// forks the thread's history-at-start (snapshot) and runs alongside any
    /// in-flight turn instead of interrupting or queueing — its result is
    /// appended to the conversation on completion (snapshot + append).
    Parallel,
}

impl Default for QueueMode {
    fn default() -> Self {
        Self::Interrupt
    }
}

impl fmt::Display for QueueMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupt => write!(f, "interrupt"),
            Self::Steer => write!(f, "steer"),
            Self::Followup => write!(f, "followup"),
            Self::Collect => write!(f, "collect"),
            Self::Parallel => write!(f, "parallel"),
        }
    }
}

/// A message sitting in the run queue, tagged with its lane.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub text: String,
    pub mode: QueueMode,
    pub client_id: String,
    pub thread_id: String,
    pub queued_at_ms: u64,
    pub model_override: Option<String>,
    pub temperature: Option<f64>,
    pub profile_id: Option<String>,
    pub locale: Option<String>,
}

/// Snapshot of the queue state for introspection.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueStatus {
    pub steers: usize,
    pub followups: usize,
    pub collects: usize,
    pub total: usize,
}
