use std::path::{Path, PathBuf};
use std::time::Duration;

use tinymcp::{ProbeOutcome, ServerRef, SupervisorEvent, TickReport};

use super::*;
use crate::core::events::DomainEvent;

/// The workspace a tick is attributed to. One process supervises every
/// workspace it has opened, so every event has to name the one it came from.
fn workspace() -> &'static Path {
    Path::new("/tmp/openhuman-ws-a")
}

fn server() -> ServerRef {
    ServerRef {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        display_name: "Inference".into(),
    }
}

#[test]
fn an_answered_probe_is_not_an_event() {
    // One row per server per minute would bury everything else in the log.
    let events = domain_events_for(
        workspace(),
        &[SupervisorEvent::ProbeAnswered {
            server: server(),
            elapsed: Duration::from_millis(190),
        }],
    );
    assert!(events.is_empty());
}

#[test]
fn a_kept_timeout_becomes_probe_timed_out_with_its_place_in_the_streak() {
    let events = domain_events_for(
        workspace(),
        &[SupervisorEvent::ProbeTimedOut {
            server: server(),
            after: Duration::from_secs(8),
            consecutive: 1,
            teardown_after: 3,
        }],
    );
    match events.as_slice() {
        [DomainEvent::McpServerProbeTimedOut {
            server_id,
            qualified_name,
            probe_timeout_secs,
            consecutive_timeouts,
            teardown_after,
            ..
        }] => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(qualified_name, "ac.inference.sh/mcp");
            assert_eq!(*probe_timeout_secs, 8);
            assert_eq!(*consecutive_timeouts, 1);
            assert_eq!(*teardown_after, 3);
        }
        other => panic!("expected one McpServerProbeTimedOut, got {other:?}"),
    }
    assert_eq!(events[0].domain(), "mcp_client");
    assert_eq!(events[0].agent_hint(), Some("ac.inference.sh/mcp"));
}

#[test]
fn a_broken_drop_carries_the_error_and_how_long_it_took_to_fail() {
    let events = domain_events_for(
        workspace(),
        &[SupervisorEvent::TransportDropped {
            server: server(),
            outcome: ProbeOutcome::Broken {
                error: "mcp transport failure for `https://api.inference.sh`: connection reset"
                    .into(),
                elapsed: Duration::from_millis(1961),
            },
            consecutive_timeouts: 0,
        }],
    );
    match events.as_slice() {
        [DomainEvent::McpServerTransportDropped {
            outcome,
            detail,
            elapsed_ms,
            consecutive_timeouts,
            ..
        }] => {
            assert_eq!(outcome, "broken");
            assert_eq!(
                detail.as_deref(),
                Some("mcp transport failure for `https://api.inference.sh`: connection reset")
            );
            assert_eq!(*elapsed_ms, Some(1961));
            assert_eq!(*consecutive_timeouts, 0);
        }
        other => panic!("expected one McpServerTransportDropped, got {other:?}"),
    }
}

#[test]
fn a_timeout_drop_carries_the_window_and_the_streak_that_ended_the_session() {
    let events = domain_events_for(
        workspace(),
        &[SupervisorEvent::TransportDropped {
            server: server(),
            outcome: ProbeOutcome::TimedOut {
                after: Duration::from_secs(8),
            },
            consecutive_timeouts: 3,
        }],
    );
    match events.as_slice() {
        [DomainEvent::McpServerTransportDropped {
            outcome,
            detail,
            elapsed_ms,
            consecutive_timeouts,
            ..
        }] => {
            assert_eq!(outcome, "timed_out");
            assert_eq!(*detail, None);
            assert_eq!(*elapsed_ms, Some(8_000));
            assert_eq!(*consecutive_timeouts, 3);
        }
        other => panic!("expected one McpServerTransportDropped, got {other:?}"),
    }
}

#[test]
fn a_missing_entry_drop_carries_nothing_measured() {
    let events = domain_events_for(
        workspace(),
        &[SupervisorEvent::TransportDropped {
            server: server(),
            outcome: ProbeOutcome::Missing,
            consecutive_timeouts: 0,
        }],
    );
    match events.as_slice() {
        [DomainEvent::McpServerTransportDropped {
            outcome,
            detail,
            elapsed_ms,
            ..
        }] => {
            assert_eq!(outcome, "missing");
            assert_eq!(*detail, None);
            assert_eq!(*elapsed_ms, None);
        }
        other => panic!("expected one McpServerTransportDropped, got {other:?}"),
    }
}

#[test]
fn a_reconnect_a_failure_and_a_parking_translate_in_order() {
    let events = domain_events_for(
        workspace(),
        &[
            SupervisorEvent::Reconnected {
                server: server(),
                tools: 25,
                after_failures: 2,
            },
            SupervisorEvent::ReconnectFailed {
                server: server(),
                error: "mcp error response: not accepting sessions".into(),
                failures: 1,
                retry_in: Duration::from_secs(5),
            },
            SupervisorEvent::Parked {
                server: server(),
                error: "the `npx` launcher is not installed".into(),
            },
        ],
    );

    match events.as_slice() {
        [DomainEvent::McpServerReconnected {
            server_id,
            tool_count,
            after_failures,
            ..
        }, DomainEvent::McpServerReconnectFailed {
            error,
            failures,
            retry_in_secs,
            ..
        }, DomainEvent::McpServerParked {
            qualified_name,
            error: parked_error,
            ..
        }] => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(*tool_count, 25);
            assert_eq!(*after_failures, 2);
            assert_eq!(error, "mcp error response: not accepting sessions");
            assert_eq!(*failures, 1);
            assert_eq!(*retry_in_secs, 5);
            assert_eq!(qualified_name, "ac.inference.sh/mcp");
            assert_eq!(parked_error, "the `npx` launcher is not installed");
        }
        other => panic!("expected reconnected, reconnect_failed, parked; got {other:?}"),
    }
    let names: Vec<_> = events.iter().map(DomainEvent::variant_name).collect();
    assert_eq!(
        names,
        [
            "McpServerReconnected",
            "McpServerReconnectFailed",
            "McpServerParked"
        ]
    );
}

#[test]
fn publish_counts_only_what_became_an_event() {
    let report = TickReport {
        events: vec![
            SupervisorEvent::ProbeAnswered {
                server: server(),
                elapsed: Duration::from_millis(200),
            },
            SupervisorEvent::ReconnectFailed {
                server: server(),
                error: "connection refused".into(),
                failures: 1,
                retry_in: Duration::from_secs(5),
            },
        ],
    };
    assert_eq!(publish(workspace(), &report), 1);
    assert_eq!(publish(workspace(), &TickReport::default()), 0);
}

/// Every translated event names the workspace whose host was ticked.
///
/// The supervisor walks every workspace the process has opened, so an event
/// that did not say which one it came from would let a switched-away
/// account's outage be stored in, and announced from, the current one
/// (#5931).
#[test]
fn every_event_is_stamped_with_the_workspace_it_came_from() {
    let events = domain_events_for(
        workspace(),
        &[
            SupervisorEvent::ProbeTimedOut {
                server: server(),
                after: Duration::from_secs(8),
                consecutive: 1,
                teardown_after: 3,
            },
            SupervisorEvent::TransportDropped {
                server: server(),
                outcome: ProbeOutcome::Missing,
                consecutive_timeouts: 0,
            },
            SupervisorEvent::Reconnected {
                server: server(),
                tools: 25,
                after_failures: 2,
            },
            SupervisorEvent::ReconnectFailed {
                server: server(),
                error: "connection refused".into(),
                failures: 1,
                retry_in: Duration::from_secs(5),
            },
            SupervisorEvent::Parked {
                server: server(),
                error: "the `npx` launcher is not installed".into(),
            },
        ],
    );

    let expected = PathBuf::from(workspace());
    let stamps: Vec<&PathBuf> = events
        .iter()
        .map(|event| match event {
            DomainEvent::McpServerProbeTimedOut { workspace_dir, .. }
            | DomainEvent::McpServerTransportDropped { workspace_dir, .. }
            | DomainEvent::McpServerReconnected { workspace_dir, .. }
            | DomainEvent::McpServerReconnectFailed { workspace_dir, .. }
            | DomainEvent::McpServerParked { workspace_dir, .. } => workspace_dir,
            other => panic!("not a supervisor event: {other:?}"),
        })
        .collect();
    assert_eq!(stamps.len(), 5);
    assert!(stamps.iter().all(|stamp| **stamp == expected));
}

/// A second workspace's tick is stamped with *its* workspace, not the first's.
#[test]
fn two_workspaces_ticked_in_one_pass_keep_their_own_stamps() {
    let other = Path::new("/tmp/openhuman-ws-b");
    let one = domain_events_for(
        workspace(),
        &[SupervisorEvent::Parked {
            server: server(),
            error: "the `npx` launcher is not installed".into(),
        }],
    );
    let two = domain_events_for(
        other,
        &[SupervisorEvent::Parked {
            server: server(),
            error: "the `npx` launcher is not installed".into(),
        }],
    );

    match (one.as_slice(), two.as_slice()) {
        (
            [DomainEvent::McpServerParked {
                workspace_dir: first,
                ..
            }],
            [DomainEvent::McpServerParked {
                workspace_dir: second,
                ..
            }],
        ) => {
            assert_eq!(first, workspace());
            assert_eq!(second, other);
            assert_ne!(first, second);
        }
        other => panic!("expected one parked event each, got {other:?}"),
    }
}
