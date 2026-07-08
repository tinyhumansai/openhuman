//! End-to-end coverage for the ChannelHost capability boundary.
//!
//! Builds the *real* OpenHuman `ChannelHost` via `build_channel_host` and
//! drives each capability through the `dyn ChannelHost` trait objects exactly
//! as a ported channel provider would — proving the assembled host works
//! across the portable boundary, not just the concrete adapters in isolation.

use std::sync::Arc;

use openhuman_core::openhuman::channels::host::build_channel_host;
use openhuman_core::openhuman::config::Config;
use tinychannels::host::{ApprovalDecision, ConversationMessage, ReactionQuery};

/// A config pinned to a throwaway workspace with the local runtime disabled,
/// so filesystem-backed capabilities are isolated and inference is short-circuited.
fn test_config(workspace: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.local_ai.runtime_enabled = false;
    config
}

#[test]
fn host_advertises_the_wired_capability_set() {
    let host = build_channel_host(Arc::new(Config::default()));
    let caps = host.capabilities();
    assert!(caps.lifecycle);
    assert!(caps.stt);
    assert!(caps.tts);
    assert!(caps.reaction_gate);
    assert!(caps.approvals);
    assert!(caps.conversation_store);
    assert!(caps.event_sink);
    assert!(!caps.is_lean());
    // Not yet backed portably — a provider needing these degrades gracefully.
    assert!(!caps.turn_dispatch);
    assert!(!caps.run_ledger);
}

#[tokio::test]
async fn conversation_store_roundtrips_through_the_boundary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = build_channel_host(Arc::new(test_config(dir.path())));
    let store = host.conversations().expect("conversation store present");

    let thread = "e2e-thread";
    store
        .append(
            thread,
            ConversationMessage {
                role: "user".into(),
                content: "ping".into(),
                timestamp: None,
            },
        )
        .await
        .expect("append user");
    store
        .append(
            thread,
            ConversationMessage {
                role: "assistant".into(),
                content: "pong".into(),
                timestamp: None,
            },
        )
        .await
        .expect("append assistant");

    let history = store.history(thread, 10).await.expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "ping");
    assert_eq!(history[1].content, "pong");
}

#[tokio::test]
async fn reaction_gate_short_circuits_when_runtime_disabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = build_channel_host(Arc::new(test_config(dir.path())));
    let gate = host.reactions().expect("reaction gate present");

    let decision = gate
        .should_react(ReactionQuery {
            message: "hello there".into(),
            channel_type: "web".into(),
        })
        .await
        .expect("should_react ok");
    assert!(!decision.should_react);
    assert!(decision.emoji.is_none());
}

#[test]
fn approval_gate_parses_replies_through_the_boundary() {
    let host = build_channel_host(Arc::new(Config::default()));
    let gate = host.approvals().expect("approval gate present");
    assert_eq!(gate.parse_reply("yes"), Some(ApprovalDecision::Approve));
    assert_eq!(gate.parse_reply("no"), Some(ApprovalDecision::Deny));
    assert_eq!(gate.parse_reply("not-a-decision"), None);
}

#[tokio::test]
async fn event_sink_publishes_web_channel_events() {
    let host = build_channel_host(Arc::new(Config::default()));
    let events = host.events().expect("event sink present");
    let ok = events
        .publish(
            "web",
            "chat_segment",
            serde_json::json!({
                "event": "chat_segment",
                "client_id": "c",
                "thread_id": "t",
                "request_id": "r",
                "message": "hi"
            }),
        )
        .await;
    assert!(ok.is_ok());
}

#[test]
fn lifecycle_registry_accepts_shutdown_hooks() {
    let host = build_channel_host(Arc::new(Config::default()));
    let lifecycle = host.lifecycle().expect("lifecycle present");
    // Registration must not panic; the hook runs only at real process shutdown.
    lifecycle.register_shutdown(
        "e2e-test-hook",
        Box::new(|| Box::pin(async move { /* no-op */ })),
    );
}
