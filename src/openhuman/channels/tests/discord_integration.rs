//! Integration tests proving the channels module is fully encapsulated for
//! the Discord dispatch path.
//!
//! "Fully encapsulated" here means: the runtime dispatch pipeline can be
//! exercised end-to-end for `channel = "discord"` with every cross-module
//! boundary (agent runtime, memory backend, LLM provider) substituted with a
//! stub/noop. These tests do NOT spin up a real Discord gateway, a real LLM
//! provider, or a real memory store — they only exercise the channels module
//! itself.
//!
//! Coverage:
//!   1. End-to-end dispatch for a Discord inbound message via the real
//!      `agent.run_turn` bus handler (full pipeline smoke test).
//!   2. Discord channels report `supports_reactions() == false`, so dispatch
//!      must NOT emit a `[REACTION:<emoji>]` acknowledgment even when the
//!      inbound carries a `thread_ts`.
//!   3. Discord follows standard non-Telegram semantics: different
//!      `thread_ts` values produce independent conversation histories at the
//!      dispatch level (not just at the key function level).
//!   4. The dispatch path for Discord routes through the `agent.run_turn`
//!      bus handler — proved by overriding it with a stub and asserting the
//!      stub is invoked. This is the encapsulation money shot: if dispatch
//!      ever reverts to calling `run_tool_call_loop` directly, this test
//!      starts failing.

use super::super::context::{
    conversation_history_key, ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS,
};
use super::super::runtime::process_channel_message;
use super::super::traits;
use super::super::{Channel, SendMessage};
use super::common::{HistoryCaptureProvider, NoopMemory};
use crate::openhuman::agent::bus::{mock_agent_run_turn, AgentTurnResponse};
use crate::openhuman::providers::{ChatMessage, Provider};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ── Test helpers ────────────────────────────────────────────────────────────

/// A full-recording Discord channel that captures every send, start_typing,
/// and stop_typing call. Reports `name() == "discord"` and leaves
/// `supports_reactions()` at its trait default of `false` — mirroring the
/// real `DiscordChannel`. No HTTP is involved.
#[derive(Default)]
struct DiscordRecordingChannel {
    sent: tokio::sync::Mutex<Vec<SendMessage>>,
    start_typing_calls: AtomicUsize,
    stop_typing_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Channel for DiscordRecordingChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent.lock().await.push(message.clone());
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.start_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.stop_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    // Intentionally left at the default `supports_reactions() -> false` so we
    // can prove dispatch honors that capability for Discord.
}

/// Provider that immediately returns a fixed response string — the channels
/// module never needs to know or care that it's not a real LLM.
struct FixedResponseProvider {
    response: &'static str,
}

#[async_trait::async_trait]
impl Provider for FixedResponseProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(self.response.to_string())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(self.response.to_string())
    }
}

fn make_discord_ctx(
    channel: Arc<dyn Channel>,
    provider: Arc<dyn Provider>,
) -> Arc<ChannelRuntimeContext> {
    let mut channels = HashMap::new();
    channels.insert(channel.name().to_string(), channel);

    Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels),
        provider,
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 1,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options: crate::openhuman::providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    })
}

// ── 1. Full-pipeline smoke test ─────────────────────────────────────────────

/// A Discord inbound message must flow through the full runtime dispatch
/// pipeline — memory lookup, history update, `agent.run_turn` bus call,
/// channel send — without requiring any external services. The response text
/// from the stubbed provider must reach the channel's `send()` with the
/// recipient matching `reply_target`.
#[tokio::test]
async fn discord_inbound_dispatches_through_full_pipeline() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let recorder = Arc::new(DiscordRecordingChannel::default());
    let channel: Arc<dyn Channel> = recorder.clone();
    let provider: Arc<dyn Provider> = Arc::new(FixedResponseProvider {
        response: "hi from discord",
    });
    let ctx = make_discord_ctx(channel, provider);

    process_channel_message(
        ctx,
        traits::ChannelMessage {
            id: "discord_msg_1".to_string(),
            sender: "user-123".to_string(),
            reply_target: "channel-456".to_string(),
            content: "what's up?".to_string(),
            channel: "discord".to_string(),
            timestamp: 1,
            thread_ts: None,
        },
    )
    .await;

    let sent = recorder.sent.lock().await;
    assert_eq!(
        sent.len(),
        1,
        "expected exactly one send for discord dispatch"
    );
    assert_eq!(
        sent[0].recipient, "channel-456",
        "recipient must match reply_target"
    );
    assert!(
        sent[0].content.contains("hi from discord"),
        "dispatch must forward the stubbed provider response verbatim, got {:?}",
        sent[0].content
    );
    assert!(
        sent[0].thread_ts.is_none(),
        "absent inbound thread_ts must not be fabricated on discord send"
    );
}

// ── 2. Reaction capability flag is respected ───────────────────────────────

/// Dispatch must NOT emit an acknowledgment `[REACTION:<emoji>]` for Discord
/// even when the inbound message has `thread_ts` set, because Discord
/// channels report `supports_reactions() == false`. This proves the
/// dispatcher respects channel capability flags and keeps Discord free of
/// Telegram-specific behaviors.
#[tokio::test]
async fn discord_threaded_message_does_not_emit_reaction_ack() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let recorder = Arc::new(DiscordRecordingChannel::default());
    let channel: Arc<dyn Channel> = recorder.clone();
    let provider: Arc<dyn Provider> = Arc::new(FixedResponseProvider { response: "roger" });
    let ctx = make_discord_ctx(channel, provider);

    process_channel_message(
        ctx,
        traits::ChannelMessage {
            id: "discord_msg_2".to_string(),
            sender: "user-123".to_string(),
            reply_target: "channel-456".to_string(),
            content: "in-thread message".to_string(),
            channel: "discord".to_string(),
            timestamp: 1,
            thread_ts: Some("thread-42".to_string()),
        },
    )
    .await;

    let sent = recorder.sent.lock().await;
    // Only the real reply should be sent — no acknowledgment reaction.
    assert_eq!(
        sent.len(),
        1,
        "Discord must not receive an ack reaction alongside the reply"
    );
    assert!(
        !sent[0].content.starts_with("[REACTION:"),
        "discord send must not contain a reaction marker, got {:?}",
        sent[0].content
    );
    assert!(
        sent[0].content.contains("roger"),
        "expected the normal reply content, got {:?}",
        sent[0].content
    );
}

// ── 3. thread_ts splits history at the dispatch level ─────────────────────

/// Discord follows the standard non-Telegram history rules: two messages
/// with different `thread_ts` values must produce two independent
/// conversation histories. The second call's history must NOT contain the
/// first message's user content — proving the thread split is honored by
/// the actual dispatch pipeline, not just by `conversation_history_key` in
/// isolation.
#[tokio::test]
async fn discord_thread_ts_splits_conversation_history_end_to_end() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let recorder = Arc::new(DiscordRecordingChannel::default());
    let channel: Arc<dyn Channel> = recorder.clone();
    let provider_impl = Arc::new(HistoryCaptureProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let ctx = make_discord_ctx(channel, provider);

    let first = traits::ChannelMessage {
        id: "discord_msg_a".to_string(),
        sender: "user-123".to_string(),
        reply_target: "channel-456".to_string(),
        content: "first thread message".to_string(),
        channel: "discord".to_string(),
        timestamp: 1,
        thread_ts: Some("thread-A".to_string()),
    };

    let second = traits::ChannelMessage {
        id: "discord_msg_b".to_string(),
        thread_ts: Some("thread-B".to_string()),
        content: "second thread message".to_string(),
        ..first.clone()
    };

    // Sanity: the key function itself must split these. Without this, the
    // end-to-end expectations below would be ambiguous — is the split
    // happening because of the key or because of some dispatch quirk?
    assert_ne!(
        conversation_history_key(&first),
        conversation_history_key(&second),
        "discord: different thread_ts must produce different history keys"
    );

    process_channel_message(ctx.clone(), first).await;
    process_channel_message(ctx, second).await;

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        calls.len(),
        2,
        "expected two provider calls, one per thread"
    );

    // Second call's history must be fresh — only system + its own user
    // message — because it's in a brand new thread.
    let second_history = &calls[1];
    assert_eq!(
        second_history.len(),
        2,
        "thread-B history should only contain system + its own user message, got {:?}",
        second_history
    );
    assert_eq!(second_history[0].0, "system");
    assert_eq!(second_history[1].0, "user");
    assert!(
        second_history[1].1.contains("second thread message"),
        "second slot must contain the thread-B user message, got {:?}",
        second_history[1].1
    );
    assert!(
        !second_history
            .iter()
            .any(|(_, content)| content.contains("first thread message")),
        "thread-B history MUST NOT leak content from thread-A"
    );
}

// ── 4. Encapsulation money shot: stub the agent bus handler ────────────────

/// Full encapsulation proof: install a stub `agent.run_turn` bus handler,
/// drive a Discord message end-to-end, assert the stub was called exactly
/// once and its canned response reached the channel. This is the end-to-end
/// coverage that closes the decoupling loop for the Discord dispatch path —
/// if dispatch ever reverts to calling `run_tool_call_loop` directly, this
/// test starts failing because the stub handler won't be invoked.
#[tokio::test]
async fn discord_dispatch_routes_through_agent_run_turn_bus_handler() {
    // Install a stub `agent.run_turn` handler via the shared mock bus
    // helper. The returned guard holds `BUS_HANDLER_LOCK` for the whole
    // test body and re-registers production handlers on drop — even on
    // panic — so no manual restore call is required.
    let stub_calls = Arc::new(AtomicUsize::new(0));
    let stub_calls_for_handler = Arc::clone(&stub_calls);
    let _bus_guard = mock_agent_run_turn(move |req| {
        let stub_calls = Arc::clone(&stub_calls_for_handler);
        async move {
            stub_calls.fetch_add(1, Ordering::SeqCst);
            // Sanity-check the payload the dispatcher built for us.
            assert_eq!(req.channel_name, "discord");
            assert_eq!(req.provider_name, "test-provider");
            assert_eq!(req.model, "test-model");
            assert!(
                req.history.len() >= 2,
                "history should include at least the system prompt and user message"
            );
            Ok(AgentTurnResponse {
                text: "CANNED_DISCORD_RESPONSE".to_string(),
            })
        }
    })
    .await;

    let recorder = Arc::new(DiscordRecordingChannel::default());
    let channel: Arc<dyn Channel> = recorder.clone();
    // Minimal provider — never invoked because the stub short-circuits.
    let ctx = make_discord_ctx(channel, Arc::new(super::common::DummyProvider));

    process_channel_message(
        ctx,
        traits::ChannelMessage {
            id: "discord_stub_msg".to_string(),
            sender: "user-123".to_string(),
            reply_target: "channel-456".to_string(),
            content: "hello via stub".to_string(),
            channel: "discord".to_string(),
            timestamp: 1,
            thread_ts: None,
        },
    )
    .await;

    assert_eq!(
        stub_calls.load(Ordering::SeqCst),
        1,
        "discord dispatch must route through the agent.run_turn bus handler exactly once"
    );

    let sent = recorder.sent.lock().await;
    assert_eq!(sent.len(), 1, "stubbed response must reach the channel");
    assert!(
        sent[0].content.contains("CANNED_DISCORD_RESPONSE"),
        "delivered message should contain the stubbed text, got {:?}",
        sent[0].content
    );
}
