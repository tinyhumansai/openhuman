//! Turning what the reconnect supervisor observed into this application's
//! events.
//!
//! `tinymcp::Supervisor::tick` hands back a [`TickReport`] and publishes
//! nothing itself: which of its observations a user should hear about is the
//! host's call. This module makes that call once. Every non-nominal
//! observation becomes a [`DomainEvent`] in the `mcp_client` domain, which is
//! what puts it on the developer Event Log (`GET /events/domain`) and in front
//! of the notification bridge (`desktop::notifications::bus`), where the
//! stays-down, restored and parked cases become user notifications (#5931).
//!
//! Every event is stamped with the workspace whose host was ticked. One
//! process supervises every workspace it has opened over its life, and a
//! workspace switch leaves the old one open and still supervised, so a
//! subscriber that persists one of these — the notification bridge — needs
//! the stamp to file it under the right workspace. It cannot infer that from
//! its own binding: the bridge is registered once, with whichever workspace
//! booted, which may no longer be the one the user is in.
//!
//! An answered probe is the nominal case and is deliberately *not* an event:
//! one row per server per minute would bury everything else in the log.
//! `tinymcp` logs it at trace level, and that is enough.

use std::path::Path;

use tinymcp::{ProbeOutcome, SupervisorEvent, TickReport};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

const LOG_PREFIX: &str = "[mcp]";

/// The domain events a tick's observations translate to, in the order they
/// were observed.
///
/// Pure, so the mapping is testable without a bus. Observations this host has
/// no event for — an answered probe today, and whatever `tinymcp` adds to its
/// non-exhaustive report later — map to nothing.
///
/// `workspace` is the host's workspace, stamped onto every event so a
/// subscriber can reject one that is not its own.
pub fn domain_events_for(workspace: &Path, events: &[SupervisorEvent]) -> Vec<DomainEvent> {
    events
        .iter()
        .filter_map(|event| domain_event_for(workspace, event))
        .collect()
}

fn domain_event_for(workspace: &Path, event: &SupervisorEvent) -> Option<DomainEvent> {
    let server = event.server();
    let server_id = server.server_id.clone();
    let qualified_name = server.qualified_name.clone();
    let workspace_dir = workspace.to_path_buf();

    match event {
        SupervisorEvent::ProbeAnswered { .. } => None,
        SupervisorEvent::ProbeTimedOut {
            after,
            consecutive,
            teardown_after,
            ..
        } => Some(DomainEvent::McpServerProbeTimedOut {
            server_id,
            qualified_name,
            probe_timeout_secs: after.as_secs(),
            consecutive_timeouts: *consecutive,
            teardown_after: *teardown_after,
            workspace_dir,
        }),
        SupervisorEvent::TransportDropped {
            outcome,
            consecutive_timeouts,
            ..
        } => {
            let (detail, elapsed_ms) = match outcome {
                ProbeOutcome::Broken { error, elapsed } => {
                    (Some(error.clone()), Some(millis(*elapsed)))
                }
                ProbeOutcome::TimedOut { after } => (None, Some(millis(*after))),
                // `Missing` measured nothing; `Alive` never drops a session;
                // anything `tinymcp` adds later carries no detail we know of.
                _ => (None, None),
            };
            Some(DomainEvent::McpServerTransportDropped {
                server_id,
                qualified_name,
                outcome: outcome.as_str().to_string(),
                detail,
                elapsed_ms,
                consecutive_timeouts: *consecutive_timeouts,
                workspace_dir,
            })
        }
        SupervisorEvent::Reconnected {
            tools,
            after_failures,
            ..
        } => Some(DomainEvent::McpServerReconnected {
            server_id,
            qualified_name,
            tool_count: u32::try_from(*tools).unwrap_or(u32::MAX),
            after_failures: *after_failures,
            workspace_dir,
        }),
        SupervisorEvent::ReconnectFailed {
            error,
            failures,
            retry_in,
            ..
        } => Some(DomainEvent::McpServerReconnectFailed {
            server_id,
            qualified_name,
            error: error.clone(),
            failures: *failures,
            retry_in_secs: retry_in.as_secs(),
            workspace_dir,
        }),
        SupervisorEvent::Parked { error, .. } => Some(DomainEvent::McpServerParked {
            server_id,
            qualified_name,
            error: error.clone(),
            workspace_dir,
        }),
        // `SupervisorEvent` is non-exhaustive: a report entry this host does
        // not know yet is logged and skipped rather than mistranslated.
        other => {
            tracing::debug!(
                kind = other.kind(),
                server_id = %server_id,
                "{LOG_PREFIX} supervisor observation has no domain event; skipping"
            );
            None
        }
    }
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Publishes the domain events for one tick's report on the bus, in order.
///
/// Returns how many were published: zero for a quiet tick. A bus that is not
/// initialised drops them, as it does every other publish before startup.
///
/// `workspace` is the host the report came from, stamped onto every event.
pub fn publish(workspace: &Path, report: &TickReport) -> usize {
    let events = domain_events_for(workspace, &report.events);
    let published = events.len();
    if published > 0 {
        tracing::debug!(
            observations = report.events.len(),
            published,
            "{LOG_PREFIX} supervisor tick published domain events"
        );
    }
    for event in events {
        BUS.publish(event);
    }
    published
}

#[cfg(test)]
#[path = "supervisor_events_tests.rs"]
mod tests;
