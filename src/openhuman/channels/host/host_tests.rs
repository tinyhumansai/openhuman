//! Unit tests for the OpenHuman ChannelHost capability adapters.

use super::*;
use std::sync::Arc;
use tinychannels::host::{
    ApprovalDecision, ApprovalGate, ConversationMessage, ConversationStore, EventSink,
    ReactionGate, ReactionQuery, Transcriber,
};

use crate::openhuman::config::Config;

// --- ApprovalGate::parse_reply -------------------------------------------

#[test]
fn approval_gate_maps_core_replies() {
    let gate = CoreApprovalGate;
    assert_eq!(gate.parse_reply("yes"), Some(ApprovalDecision::Approve));
    assert_eq!(gate.parse_reply("APPROVE"), Some(ApprovalDecision::Approve));
    assert_eq!(gate.parse_reply("no"), Some(ApprovalDecision::Deny));
    assert_eq!(gate.parse_reply("deny"), Some(ApprovalDecision::Deny));
    assert_eq!(gate.parse_reply("banana"), None);
}

// --- ReactionGate (runtime-disabled short-circuit) ------------------------

#[tokio::test]
async fn reaction_gate_returns_default_when_runtime_disabled() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let gate = InferenceReactionGate {
        config: Arc::new(config),
    };
    let decision = gate
        .should_react(ReactionQuery {
            message: "hello".into(),
            channel_type: "web".into(),
        })
        .await
        .expect("should_react ok");
    assert!(!decision.should_react);
    assert!(decision.emoji.is_none());
}

// --- OpenHumanEventSink --------------------------------------------------

#[tokio::test]
async fn event_sink_accepts_web_channel_event_shape() {
    let sink = OpenHumanEventSink;
    let ok = sink
        .publish(
            "web",
            "chat_done",
            serde_json::json!({
                "event": "chat_done",
                "client_id": "c1",
                "thread_id": "t1",
                "request_id": "r1",
                "full_response": "hi"
            }),
        )
        .await;
    assert!(ok.is_ok());
}

#[tokio::test]
async fn event_sink_rejects_non_object_payload() {
    let sink = OpenHumanEventSink;
    let err = sink
        .publish("web", "bad", serde_json::json!([1, 2, 3]))
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn event_sink_routes_channel_reactions_to_domain_bus() {
    let sink = OpenHumanEventSink;
    assert!(sink
        .publish(
            "channel",
            "reaction_received",
            serde_json::json!({
                "channel": "telegram", "sender": "u1",
                "target_message_id": "telegram_1_2", "emoji": "👍"
            }),
        )
        .await
        .is_ok());
    assert!(sink
        .publish(
            "channel",
            "reaction_sent",
            serde_json::json!({
                "channel": "telegram", "target_message_id": "telegram_1_2",
                "emoji": "👍", "success": true
            }),
        )
        .await
        .is_ok());
    // Unknown domain/kind is a benign no-op.
    assert!(sink
        .publish("mystery", "x", serde_json::json!({}))
        .await
        .is_ok());
    assert!(sink
        .publish("channel", "unknown", serde_json::json!({}))
        .await
        .is_ok());
}

// --- ConversationHistoryStore --------------------------------------------

#[tokio::test]
async fn conversation_store_append_then_history_roundtrips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ConversationHistoryStore {
        workspace_dir: dir.path().to_path_buf(),
    };
    let thread = "thread-1";

    store
        .append(
            thread,
            ConversationMessage {
                role: "user".into(),
                content: "first".into(),
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
                content: "second".into(),
                timestamp: None,
            },
        )
        .await
        .expect("append assistant");

    let history = store.history(thread, 10).await.expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "first");
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].content, "second");
    assert_eq!(history[1].role, "assistant");

    // `limit` keeps the most recent messages.
    let recent = store.history(thread, 1).await.expect("history limited");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].content, "second");
}

// --- build_channel_host ---------------------------------------------------

#[test]
fn build_channel_host_advertises_expected_capabilities() {
    let host = build_channel_host(Arc::new(Config::default()));
    let caps = host.capabilities();
    assert!(caps.lifecycle);
    assert!(caps.stt);
    assert!(caps.tts);
    assert!(caps.reaction_gate);
    assert!(caps.approvals);
    assert!(caps.conversation_store);
    assert!(caps.event_sink);
    assert!(caps.allowlist_store);
    // Not yet backed portably:
    assert!(!caps.turn_dispatch);
    assert!(!caps.run_ledger);
    assert!(!caps.memory_recall);
}

#[test]
fn transcriber_reports_stable_name() {
    let stt = VoiceTranscriber {
        config: Arc::new(Config::default()),
    };
    assert_eq!(stt.name(), "openhuman-voice");
}
