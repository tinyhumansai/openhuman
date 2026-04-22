use super::common::DummyProvider;
use super::super::context::{
    compact_sender_history, conversation_history_key, effective_channel_message_timeout_secs,
    is_context_window_overflow_error, should_skip_memory_context_entry, ChannelRuntimeContext,
    CHANNEL_HISTORY_COMPACT_CONTENT_CHARS, CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES,
    CHANNEL_MESSAGE_TIMEOUT_SECS, MIN_CHANNEL_MESSAGE_TIMEOUT_SECS,
};
use super::super::traits;
use crate::openhuman::providers::ChatMessage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[test]
fn effective_channel_message_timeout_secs_clamps_to_minimum() {
    assert_eq!(
        effective_channel_message_timeout_secs(0),
        MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
    );
    assert_eq!(
        effective_channel_message_timeout_secs(15),
        MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
    );
    assert_eq!(effective_channel_message_timeout_secs(300), 300);
}

#[test]
fn context_window_overflow_error_detector_matches_known_messages() {
    let overflow_err = anyhow::anyhow!(
        "OpenAI Codex stream error: Your input exceeds the context window of this model."
    );
    assert!(is_context_window_overflow_error(&overflow_err));

    let other_err =
        anyhow::anyhow!("OpenAI Codex API error (502 Bad Gateway): error code: 502");
    assert!(!is_context_window_overflow_error(&other_err));
}

#[test]
fn memory_context_skip_rules_exclude_history_blobs() {
    assert!(should_skip_memory_context_entry(
        "telegram_123_history",
        r#"[{"role":"user"}]"#
    ));
    assert!(!should_skip_memory_context_entry("telegram_123_45", "hi"));
}

#[test]
fn compact_sender_history_keeps_recent_truncated_messages() {
    let mut histories = HashMap::new();
    let sender = "telegram_u1".to_string();
    histories.insert(
        sender.clone(),
        (0..20)
            .map(|idx| {
                let content = format!("msg-{idx}-{}", "x".repeat(700));
                if idx % 2 == 0 {
                    ChatMessage::user(content)
                } else {
                    ChatMessage::assistant(content)
                }
            })
            .collect::<Vec<_>>(),
    );

    let ctx = ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(super::common::NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("system".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(histories)),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
        provider_runtime_options: crate::openhuman::providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
    };

    assert!(compact_sender_history(&ctx, &sender));

    let histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let kept = histories
        .get(&sender)
        .expect("sender history should remain");
    assert_eq!(kept.len(), CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
    assert!(kept.iter().all(|turn| {
        let len = turn.content.chars().count();
        len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS
            || (len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS + 3 && turn.content.ends_with("..."))
    }));
}

// ── conversation_history_key tests ─────────────────────────────────────────

fn make_channel_msg(channel: &str, thread_ts: Option<&str>) -> traits::ChannelMessage {
    traits::ChannelMessage {
        id: "test_id".to_string(),
        sender: "alice".to_string(),
        reply_target: "chat-1".to_string(),
        content: "hello".to_string(),
        channel: channel.to_string(),
        timestamp: 1,
        thread_ts: thread_ts.map(ToString::to_string),
    }
}

/// Telegram uses thread_ts for reply targeting only; it must not split history.
#[test]
fn telegram_history_key_is_thread_ts_agnostic() {
    let no_thread = make_channel_msg("telegram", None);
    let with_thread = make_channel_msg("telegram", Some("99"));
    let other_thread = make_channel_msg("telegram", Some("777"));

    let key_base = conversation_history_key(&no_thread);
    let key_a = conversation_history_key(&with_thread);
    let key_b = conversation_history_key(&other_thread);

    assert_eq!(key_base, key_a, "telegram: thread_ts must not change history key");
    assert_eq!(key_a, key_b, "telegram: different thread_ts must share history key");
}

/// For every other channel (e.g. Slack, Discord), thread_ts splits conversation
/// history so each thread is an independent context.
#[test]
fn non_telegram_history_key_differs_by_thread_ts() {
    let no_thread = make_channel_msg("slack", None);
    let with_thread = make_channel_msg("slack", Some("1234567890.000001"));

    let key_base = conversation_history_key(&no_thread);
    let key_thread = conversation_history_key(&with_thread);

    assert_ne!(
        key_base, key_thread,
        "non-telegram channels must produce distinct keys for different thread_ts values"
    );
}
