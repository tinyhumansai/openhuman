//! What one `composio_sync` run was for, and what it reported.
//!
//! [`SyncReason`] is the run's trigger, passed to the connector module;
//! [`SyncOutcome`] is the `openhuman.composio_sync` reply. Both are serde
//! shapes with no behaviour.

use serde::{Deserialize, Serialize};

/// Reason a sync was triggered. Providers use this to decide whether to do a
/// full backfill or an incremental pull.
///
/// The serde form is `snake_case` and is mirrored into audit rows, so the
/// variant names are a compatibility surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    /// First sync immediately after an OAuth handoff completes.
    ConnectionCreated,
    /// Periodic background sync from the scheduler.
    Periodic,
    /// Explicit user-driven sync from RPC or the UI.
    Manual,
}

impl SyncReason {
    /// Stable lowercase tag, matching the serde representation.
    ///
    /// Callers that stamp the reason into a log line or an audit row want the
    /// string without a serde round-trip; this is that string, and the pin test
    /// holds the two forms equal.
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncReason::ConnectionCreated => "connection_created",
            SyncReason::Periodic => "periodic",
            SyncReason::Manual => "manual",
        }
    }
}

/// The `openhuman.composio_sync` reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOutcome {
    /// Composio toolkit slug the run covered.
    pub toolkit: String,
    /// The connection that was synced; `None` for toolkit-wide runs.
    pub connection_id: Option<String>,
    /// Why the run happened — normally a [`SyncReason::as_str`] tag, kept as a
    /// `String` because a caller may report a reason the enum does not model.
    pub reason: String,
    /// How many items the run ingested.
    pub items_ingested: usize,
    /// Wall-clock start, epoch milliseconds.
    pub started_at_ms: u64,
    /// Wall-clock finish, epoch milliseconds.
    pub finished_at_ms: u64,
    /// One-line human summary for the status panel.
    pub summary: String,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub details: serde_json::Value,
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
