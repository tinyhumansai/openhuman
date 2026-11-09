//! [`DomainEvent::log_detail`]: the one-line summary the developer Event Log
//! shows for the variants that need one.

use super::{clip_to_chars, DomainEvent};

impl DomainEvent {
    /// A one-line summary for the developer Event Log, for the variants whose
    /// name and [`Self::agent_hint`] are not enough to act on.
    ///
    /// The Event Log envelope (`GET /events/domain`) carries the domain, the
    /// variant name, the agent hint and a timestamp — no payload. A variant
    /// whose whole point is a failure *reason* would otherwise reach the UI
    /// with the reason discarded: a transport that broke and one that timed
    /// out are the same row (#5931). This is the opt-in escape hatch. A
    /// variant with something a reader needs returns a short rendering of it;
    /// every other variant returns `None` and its row is unchanged.
    ///
    /// What comes back is shown verbatim to anyone who can open the Event Log
    /// and lands in the log's NDJSON download, so it carries no payload
    /// content, no credentials and no un-redacted endpoint — the MCP arms
    /// pass through strings `tinymcp` has already rendered and
    /// endpoint-redacted — and no `workspace_dir`, which is on the event for
    /// subscribers to filter on, not for a shared panel to print.
    #[must_use]
    pub fn log_detail(&self) -> Option<String> {
        /// Long enough for a rendered transport error, short enough that one
        /// row cannot push the rest of the log off the screen. Counted in
        /// `char`s, so a multi-byte error cannot be split mid-character.
        const MAX_DETAIL_CHARS: usize = 160;

        let clip = |text: &str| clip_to_chars(text, MAX_DETAIL_CHARS);

        match self {
            Self::McpServerProbeTimedOut {
                probe_timeout_secs,
                consecutive_timeouts,
                teardown_after,
                ..
            } => Some(format!(
                "no answer in {probe_timeout_secs}s; timeout {consecutive_timeouts} of \
                 {teardown_after} before teardown"
            )),
            Self::McpServerTransportDropped {
                outcome,
                detail,
                elapsed_ms,
                ..
            } => {
                let mut summary = format!("session ended: {outcome}");
                if let Some(elapsed_ms) = elapsed_ms {
                    summary.push_str(&format!(" after {elapsed_ms}ms"));
                }
                if let Some(detail) = detail {
                    summary.push_str(" — ");
                    summary.push_str(&clip(detail));
                }
                Some(summary)
            }
            Self::McpServerReconnected {
                tool_count,
                after_failures,
                ..
            } => Some(format!(
                "connected with {tool_count} tools after {after_failures} failed attempt(s)"
            )),
            Self::McpServerReconnectFailed {
                error,
                failures,
                retry_in_secs,
                ..
            } => Some(format!(
                "attempt {failures} failed, retrying in {retry_in_secs}s — {}",
                clip(error)
            )),
            Self::McpServerParked { error, .. } => {
                Some(format!("parked, not retrying — {}", clip(error)))
            }
            Self::InboundTurnCompleted {
                source,
                key,
                turn_id,
                event_ids,
                ..
            } => Some(format!(
                "{source}/{key} turn {turn_id}: {} event(s)",
                event_ids.len()
            )),
            Self::InboundTurnFailed {
                source,
                key,
                turn_id,
                detail,
            } => Some(format!("{source}/{key} turn {turn_id}: {}", clip(detail))),
            _ => None,
        }
    }
}
