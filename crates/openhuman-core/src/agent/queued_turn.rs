//! Host-owned payload for a message queued while an agent turn is active.
//!
//! TinyAgents owns the queue mechanics and its [`QueueLane`](tinyagents_harness::run_queue::QueueLane)
//! selects how this payload is consumed. OpenHuman retains the web and
//! orchestration metadata needed to dispatch a deferred follow-up turn.

/// A queued OpenHuman turn input.
///
/// The payload deliberately does not carry a lane: lane selection is made at
/// the host boundary when the item is pushed into TinyAgents' `RunQueue`.
#[derive(Debug, Clone)]
pub struct QueuedTurn {
    pub text: String,
    pub client_id: String,
    pub thread_id: String,
    pub queued_at_ms: u64,
    pub model_override: Option<String>,
    pub temperature: Option<f64>,
    pub locale: Option<String>,
}
