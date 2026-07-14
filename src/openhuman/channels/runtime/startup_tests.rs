//! Tests for the chat-workload resolver wired into channel runtime startup.
//!
//! Issue #3098 sub-issue 1: prior to this fix, channel runtime startup
//! always built a cloud-only provider chain and used
//! `config.default_model`, ignoring the per-workload `chat_provider`
//! routing string. These tests pin the resolver behavior so the default
//! (cloud) path is preserved for users who haven't picked a local /
//! BYOK model, and the override path activates for those who have.

use super::{resolve_chat_workload, ChatWorkloadResolution, RelayInboundMessageHandler};
use crate::openhuman::config::Config;
use tinychannels::relay::RelayInboundHandler;

fn config_with_chat_provider(s: Option<&str>) -> Config {
    let mut config = Config::default();
    config.chat_provider = s.map(str::to_string);
    config
}

#[test]
fn chat_provider_unset_resolves_to_cloud() {
    let config = config_with_chat_provider(None);
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_blank_resolves_to_cloud() {
    let config = config_with_chat_provider(Some(""));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_cloud_sentinel_resolves_to_cloud() {
    let config = config_with_chat_provider(Some("cloud"));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_openhuman_sentinel_resolves_to_cloud() {
    let config = config_with_chat_provider(Some("openhuman"));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_ollama_resolves_to_workload() {
    let config = config_with_chat_provider(Some("ollama:llama3.2"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "ollama:llama3.2");
            assert_eq!(slug, "ollama");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for ollama, got Cloud"),
    }
}

#[test]
fn chat_provider_lmstudio_resolves_to_workload() {
    let config = config_with_chat_provider(Some("lmstudio:qwen2.5:0.5b"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "lmstudio:qwen2.5:0.5b");
            assert_eq!(slug, "lmstudio");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for lmstudio"),
    }
}

#[test]
fn chat_provider_byok_slug_resolves_to_workload() {
    let config = config_with_chat_provider(Some("openai:gpt-4o"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "openai:gpt-4o");
            assert_eq!(slug, "openai");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for byok slug"),
    }
}

#[test]
fn chat_provider_claude_agent_sdk_resolves_to_workload() {
    // Bare sentinel (no colon) — slug is the full string.
    let config = config_with_chat_provider(Some("claude_agent_sdk"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "claude_agent_sdk");
            assert_eq!(slug, "claude_agent_sdk");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for claude_agent_sdk"),
    }
}

#[tokio::test]
async fn relay_inbound_handler_forwards_envelopes_to_dispatch_bus() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let handler = RelayInboundMessageHandler::new(tx);
    let envelope = tinychannels::ChannelInboundEnvelope {
        channel: tinychannels::channel::ChannelRef {
            id: "telegram".into(),
            account_id: None,
        },
        message_id: "relay-msg-1".into(),
        conversation: tinychannels::channel::ConversationRef {
            id: "chat-1".into(),
            topic_id: Some("topic-1".into()),
            ..Default::default()
        },
        sender: tinychannels::channel::SenderRef {
            id: "alice".into(),
            ..Default::default()
        },
        text: "hello from relay".into(),
        ..Default::default()
    };

    handler
        .handle(tinychannels::relay::AuthenticatedRelayInboundEvent {
            event: serde_json::to_value(envelope).expect("relay envelope json"),
            buffer_id: Some("buffer-1".into()),
            delivered_via_authenticated_relay: true,
        })
        .await
        .expect("handle relay inbound");

    let runtime_msg = rx.recv().await.expect("forwarded channel message");
    let msg = runtime_msg.message;
    assert_eq!(msg.channel, "telegram");
    assert_eq!(msg.id, "relay-msg-1");
    assert_eq!(msg.reply_target, "chat-1");
    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.content, "hello from relay");
    assert_eq!(msg.thread_ts.as_deref(), Some("topic-1"));
    let forwarded = runtime_msg
        .inbound_envelope
        .expect("relay envelope should stay attached for dispatch");
    assert_eq!(forwarded.message_id, "relay-msg-1");
    assert_eq!(forwarded.conversation.id, "chat-1");
    assert_eq!(forwarded.conversation.topic_id.as_deref(), Some("topic-1"));
}

#[tokio::test]
async fn relay_inbound_handler_rejects_malformed_envelopes() {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let handler = RelayInboundMessageHandler::new(tx);

    let error = handler
        .handle(tinychannels::relay::AuthenticatedRelayInboundEvent {
            event: serde_json::json!("not an envelope"),
            buffer_id: None,
            delivered_via_authenticated_relay: true,
        })
        .await
        .expect_err("malformed relay payload should fail");

    assert!(error.to_string().contains("invalid inbound envelope"));
}
