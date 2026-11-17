use super::*;
use super::{ProgressiveChannelTransport, ProgressiveOutputObserver};

use crate::channels::context::ChannelRuntimeContext;
use crate::channels::runtime::dispatch::RuntimeChannelMessage;
use crate::channels::runtime::{ChannelHandle, TestChannel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tinychannels_bus::channel::{
    ChannelOutboundMessage, ChannelSendOutcome, MessageReceipt, OutboundContentPart,
};
use tinychannels_bus::module_progressive::{
    ChannelOutputEvent, ProgressiveTransportRequest, ProgressiveTransportResult,
};
use tinychannels_bus::{ChannelBackendRequest, ChannelBackendResult};

#[derive(Default)]
struct RecordingOutputObserver {
    seen: tokio::sync::Mutex<Vec<(String, String, String, u64)>>,
}

#[async_trait::async_trait]
impl ProgressiveOutputObserver for RecordingOutputObserver {
    async fn observe(
        &self,
        channel: &tinychannels_bus::ChannelRef,
        conversation: &tinychannels_bus::ConversationRef,
        turn_id: &str,
        sequence: u64,
        event: &ChannelOutputEvent,
    ) -> Result<()> {
        assert!(matches!(event, ChannelOutputEvent::TextDelta { .. }));
        self.seen.lock().await.push((
            channel.account_id.clone().unwrap_or_default(),
            conversation.id.clone(),
            turn_id.into(),
            sequence,
        ));
        Ok(())
    }
}

struct RecordingChannel {
    name: String,
    sent: tokio::sync::Mutex<Vec<ChannelOutboundMessage>>,
    progressive_requests: tokio::sync::Mutex<Vec<ProgressiveTransportRequest>>,
}

impl RecordingChannel {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            sent: tokio::sync::Mutex::new(Vec::new()),
            progressive_requests: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl RecordingChannel {
    async fn send_typed_with_receipt(
        &self,
        message: &ChannelOutboundMessage,
    ) -> anyhow::Result<ChannelSendOutcome> {
        self.sent.lock().await.push(message.clone());
        Ok(ChannelSendOutcome {
            receipt: MessageReceipt {
                primary_platform_message_id: Some("telegram-message-7".into()),
                platform_message_ids: vec!["telegram-message-7".into()],
                ..Default::default()
            },
            error: Some(tinychannels_bus::channel::ChannelSendError {
                kind: tinychannels_bus::channel::SendErrorKind::Transient,
                message: "one trailing segment was not delivered".into(),
                sent_before_error: true,
                retryable: true,
                ..Default::default()
            }),
        })
    }
}

#[async_trait::async_trait]
impl ProgressiveChannelTransport for RecordingChannel {
    fn configured_name(&self) -> &str {
        &self.name
    }

    async fn dispatch_progressive(
        &self,
        request: ProgressiveTransportRequest,
    ) -> Result<ProgressiveTransportResult> {
        self.progressive_requests.lock().await.push(request.clone());
        let ProgressiveTransportRequest::SendTyped {
            channel: _channel,
            turn_id,
            channel_ref,
            conversation,
            idempotency_key,
            parts,
            ..
        } = request
        else {
            return Ok(ProgressiveTransportResult::Unavailable);
        };
        let outbound = tinychannels_bus::channel::ChannelOutboundMessage {
            channel_id: channel_ref
                .as_ref()
                .map(|reference| reference.id.clone())
                .unwrap_or_else(|| _channel.clone()),
            conversation_id: conversation
                .as_ref()
                .map(|reference| reference.id.clone())
                .unwrap_or_default(),
            channel_ref,
            conversation_ref: conversation,
            reply_to_id: None,
            thread_id: None,
            turn_id,
            idempotency_key: idempotency_key.unwrap_or_default(),
            parts,
        };
        let outcome = self
            .send_typed_with_receipt(&outbound)
            .await
            .map_err(|_| Error::failed("test channel send failed"))?;
        Ok(ProgressiveTransportResult::Outcome(Box::new(outcome)))
    }
}

fn callback_for_channels(
    channels: HashMap<String, Arc<ChannelHandle>>,
    progressive_channels: HashMap<String, Arc<dyn ProgressiveChannelTransport>>,
    output_observer: Option<Arc<dyn ProgressiveOutputObserver>>,
) -> (ChannelHostCallbacks, Arc<ChannelRuntimeContext>) {
    let context = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: None,
        default_provider: Arc::new("test-provider".into()),
        tools_registry: Arc::new(Vec::new()),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test system"),
        model: Arc::new("test-model".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: 300,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    });
    let context_slot: Arc<OnceLock<Weak<ChannelRuntimeContext>>> = Arc::new(OnceLock::new());
    assert!(context_slot.set(Arc::downgrade(&context)).is_ok());
    let progressive_slot = Arc::new(OnceLock::new());
    assert!(progressive_slot
        .set(Arc::new(
            progressive_channels
                .into_iter()
                .map(|(name, adapter)| (name, Arc::downgrade(&adapter)))
                .collect()
        ))
        .is_ok());
    let (dispatch, _receiver) = tokio::sync::mpsc::channel::<RuntimeChannelMessage>(1);
    let mut callback = ChannelHostCallbacks::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(crate::config::Config::default()),
        context_slot,
        progressive_slot,
        output_observer,
        dispatch,
    );
    callback.cancellation = tokio_util::sync::CancellationToken::new();
    (callback, context)
}

#[tokio::test]
async fn voice_media_store_validates_the_configured_account_scope_and_declines() {
    let channel = Arc::new(RecordingChannel::new("telegram:bot-a"));
    let channels: HashMap<String, Arc<ChannelHandle>> = HashMap::from([(
        channel.name.clone(),
        Arc::new(ChannelHandle::test(TestChannel::new(channel.name.clone()))),
    )]);
    let (callback, _context) = callback_for_channels(channels, HashMap::new(), None);
    let request = || tinychannels_bus::HostStoreVoiceMediaRequest {
        scope: tinychannels_bus::HostVoiceScope {
            configured_instance: "telegram:bot-a".into(),
            channel: tinychannels_bus::ChannelRef {
                id: "telegram".into(),
                account_id: Some("telegram:bot-a".into()),
            },
            conversation: tinychannels_bus::ConversationRef {
                kind: tinychannels_bus::ConversationKind::Dm,
                id: "chat-42".into(),
                ..Default::default()
            },
            turn_id: "telegram_42_7".into(),
        },
        direction: tinychannels_bus::HostVoiceMediaDirection::Inbound,
        audio_base64: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            vec![7; 200 * 1024],
        ),
        mime_type: "audio/ogg".into(),
        file_name: Some("voice.ogg".into()),
    };

    // The module persists voice media itself; the host callback only
    // validates the scope and declines (legacy compatibility seam).
    let declined = callback
        .store_voice_media(request())
        .await
        .expect_err("the host holds no durable voice media store");
    assert!(declined
        .to_string()
        .contains("stored by the channels module"));

    let mut forged = request();
    forged.scope.configured_instance = "telegram:bot-b".into();
    forged.scope.channel.account_id = Some("telegram:bot-b".into());
    assert!(callback.store_voice_media(forged).await.is_err());

    let mut oversized = request();
    oversized.audio_base64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        vec![7; 8 * 1024 * 1024 + 1],
    );
    assert!(callback.store_voice_media(oversized).await.is_err());
}

#[tokio::test]
async fn native_progressive_transport_dispatches_typed_content_through_configured_channel() {
    let other_account = Arc::new(RecordingChannel::new("telegram:bot-a"));
    let selected_account = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let channels: HashMap<String, Arc<ChannelHandle>> = HashMap::from([
        (
            other_account.name().to_owned(),
            Arc::new(ChannelHandle::test(TestChannel::new(other_account.name()))),
        ),
        (
            selected_account.name().to_owned(),
            Arc::new(ChannelHandle::test(TestChannel::new(
                selected_account.name(),
            ))),
        ),
    ]);
    let progressive_channels: HashMap<String, Arc<dyn ProgressiveChannelTransport>> =
        HashMap::from([
            (
                "telegram:bot-a".into(),
                other_account.clone() as Arc<dyn ProgressiveChannelTransport>,
            ),
            (
                "telegram:bot-b".into(),
                selected_account.clone() as Arc<dyn ProgressiveChannelTransport>,
            ),
        ]);
    let (callback, _context) = callback_for_channels(channels, progressive_channels, None);
    let parts = vec![OutboundContentPart::Content {
        part: tinychannels_bus::ContentPart::Image {
            media: tinychannels_bus::MediaReference {
                url: Some("https://media.invalid/receipt.png".into()),
                kind: tinychannels_bus::MediaKind::Image,
                ..Default::default()
            },
            caption: Some("diagram".into()),
            alt_text: None,
        },
    }];

    let result = callback
        .dispatch_controller_backend(ChannelBackendRequest::ProgressiveTransport(Box::new(
            ProgressiveTransportRequest::SendTyped {
                channel: "telegram:chat-42".into(),
                turn_id: Some("turn-11".into()),
                channel_ref: Some(tinychannels_bus::ChannelRef {
                    id: "telegram".into(),
                    account_id: Some("telegram:bot-b".into()),
                }),
                conversation: Some(tinychannels_bus::ConversationRef {
                    id: "chat-42".into(),
                    thread_id: Some("topic-9".into()),
                    ..Default::default()
                }),
                sequence: Some(3),
                segment_index: Some(1),
                idempotency_key: Some("turn-11:3:1".into()),
                parts: parts.clone(),
                text: "diagram attached".into(),
            },
        )))
        .await
        .expect("native progressive callback should dispatch through the channel adapter");

    let ChannelBackendResult::ProgressiveTransport(result) = result else {
        panic!("expected progressive transport result");
    };
    let ProgressiveTransportResult::Outcome(outcome) = *result else {
        panic!("typed native send must return the complete provider outcome");
    };
    assert_eq!(
        outcome.receipt.primary_platform_message_id.as_deref(),
        Some("telegram-message-7")
    );
    assert_eq!(outcome.receipt.platform_message_ids, ["telegram-message-7"]);
    let error = outcome
        .error
        .as_ref()
        .expect("partial send failure is preserved");
    assert_eq!(error.message, "one trailing segment was not delivered");
    assert!(error.sent_before_error);
    assert!(error.retryable);
    assert!(other_account.sent.lock().await.is_empty());
    let sent = selected_account.sent.lock().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].channel_id, "telegram");
    assert_eq!(sent[0].channel_ref.as_ref().unwrap().id, "telegram");
    assert_eq!(
        sent[0].channel_ref.as_ref().unwrap().account_id.as_deref(),
        Some("telegram:bot-b")
    );
    assert_eq!(sent[0].conversation_id, "chat-42");
    assert_eq!(sent[0].conversation_ref.as_ref().unwrap().id, "chat-42");
    assert_eq!(
        sent[0]
            .conversation_ref
            .as_ref()
            .unwrap()
            .thread_id
            .as_deref(),
        Some("topic-9")
    );
    assert_eq!(sent[0].turn_id.as_deref(), Some("turn-11"));
    // Sequence and segment are represented in the module's idempotency key.
    assert_eq!(sent[0].idempotency_key, "turn-11:3:1");
    assert_eq!(sent[0].parts, parts);
    let requests = selected_account.progressive_requests.lock().await;
    let ProgressiveTransportRequest::SendTyped {
        sequence,
        segment_index,
        ..
    } = &requests[0]
    else {
        panic!("expected the typed progressive request");
    };
    assert_eq!(*sequence, Some(3));
    assert_eq!(*segment_index, Some(1));
}

#[tokio::test]
async fn native_progressive_transport_rejects_unconfigured_account_without_provider_fallback() {
    let first_account = Arc::new(RecordingChannel::new("telegram:bot-a"));
    let second_account = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let channels: HashMap<String, Arc<ChannelHandle>> = HashMap::from([
        (
            "telegram:bot-a".into(),
            Arc::new(ChannelHandle::test(TestChannel::new(first_account.name()))),
        ),
        (
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new(second_account.name()))),
        ),
    ]);
    let progressive_channels: HashMap<String, Arc<dyn ProgressiveChannelTransport>> =
        HashMap::from([
            (
                "telegram:bot-a".into(),
                first_account.clone() as Arc<dyn ProgressiveChannelTransport>,
            ),
            (
                "telegram:bot-b".into(),
                second_account.clone() as Arc<dyn ProgressiveChannelTransport>,
            ),
        ]);
    let (callback, _context) = callback_for_channels(channels, progressive_channels, None);

    let result = callback
        .dispatch_controller_backend(ChannelBackendRequest::ProgressiveTransport(Box::new(
            ProgressiveTransportRequest::SendTyped {
                channel: "telegram:chat-42".into(),
                turn_id: Some("turn-12".into()),
                channel_ref: Some(tinychannels_bus::ChannelRef {
                    id: "telegram".into(),
                    account_id: Some("telegram:bot-missing".into()),
                }),
                conversation: Some(tinychannels_bus::ConversationRef {
                    id: "chat-42".into(),
                    ..Default::default()
                }),
                sequence: Some(0),
                segment_index: Some(0),
                idempotency_key: Some("turn-12:0:0".into()),
                parts: vec![OutboundContentPart::Content {
                    part: tinychannels_bus::ContentPart::Text {
                        text: "must not cross accounts".into(),
                        markdown_dialect: None,
                    },
                }],
                text: "must not cross accounts".into(),
            },
        )))
        .await;

    assert!(
        result.is_err(),
        "an unknown explicit account must fail closed"
    );
    assert!(first_account.sent.lock().await.is_empty());
    assert!(second_account.sent.lock().await.is_empty());
}

#[tokio::test]
async fn native_progressive_output_event_is_acknowledged_by_optional_host_observer() {
    let observer = Arc::new(RecordingOutputObserver::default());
    let adapter = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let (callback, _context) = callback_for_channels(
        HashMap::from([(
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new("telegram:bot-b"))),
        )]),
        HashMap::from([(
            "telegram:bot-b".into(),
            adapter.clone() as Arc<dyn ProgressiveChannelTransport>,
        )]),
        Some(observer.clone() as Arc<dyn ProgressiveOutputObserver>),
    );
    let result = callback
        .dispatch_controller_backend(ChannelBackendRequest::ProgressiveTransport(Box::new(
            ProgressiveTransportRequest::OutputEvent {
                channel: "telegram:bot-b".into(),
                turn_id: Some("turn-13".into()),
                channel_ref: Some(tinychannels_bus::ChannelRef {
                    id: "telegram".into(),
                    account_id: Some("telegram:bot-b".into()),
                }),
                conversation: Some(tinychannels_bus::ConversationRef {
                    id: "chat-42".into(),
                    thread_id: Some("topic-9".into()),
                    ..Default::default()
                }),
                sequence: 4,
                event: Box::new(ChannelOutputEvent::TextDelta {
                    text: "working".into(),
                    iteration: Some(1),
                }),
            },
        )))
        .await
        .expect("an unobserved native progress event is accepted by the host");
    let ChannelBackendResult::ProgressiveTransport(result) = result else {
        panic!("expected progressive transport result");
    };
    assert!(matches!(*result, ProgressiveTransportResult::Acknowledged));
    assert_eq!(
        *observer.seen.lock().await,
        [(
            "telegram:bot-b".into(),
            "chat-42".into(),
            "turn-13".into(),
            4
        )]
    );
    assert!(adapter.progressive_requests.lock().await.is_empty());
}

fn output_event_request(
    account: Option<&str>,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
) -> ChannelBackendRequest {
    ChannelBackendRequest::ProgressiveTransport(Box::new(
        ProgressiveTransportRequest::OutputEvent {
            channel: "telegram:chat-42".into(),
            turn_id: turn_id.map(str::to_owned),
            channel_ref: account.map(|account_id| tinychannels_bus::ChannelRef {
                id: "telegram".into(),
                account_id: Some(account_id.into()),
            }),
            conversation: conversation_id.map(|id| tinychannels_bus::ConversationRef {
                id: id.into(),
                ..Default::default()
            }),
            sequence: 5,
            event: Box::new(ChannelOutputEvent::TextDelta {
                text: "progress".into(),
                iteration: None,
            }),
        },
    ))
}

#[tokio::test]
async fn native_progressive_output_event_validates_scope_without_observer() {
    let adapter = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let (callback, _context) = callback_for_channels(
        HashMap::from([(
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new("telegram:bot-b"))),
        )]),
        HashMap::from([(
            "telegram:bot-b".into(),
            adapter.clone() as Arc<dyn ProgressiveChannelTransport>,
        )]),
        None,
    );
    let valid = callback
        .dispatch_controller_backend(output_event_request(
            Some("telegram:bot-b"),
            Some("chat-42"),
            Some("turn-14"),
        ))
        .await
        .expect("module output events are acknowledged when no host observer is installed");
    assert!(matches!(
        valid,
        ChannelBackendResult::ProgressiveTransport(result)
            if matches!(*result, ProgressiveTransportResult::Acknowledged)
    ));

    for invalid in [
        output_event_request(Some("telegram:missing"), Some("chat-42"), Some("turn-14")),
        output_event_request(Some("telegram:bot-b"), None, Some("turn-14")),
        output_event_request(Some("telegram:bot-b"), Some("chat-42"), None),
    ] {
        assert!(
            callback.dispatch_controller_backend(invalid).await.is_err(),
            "output event scope must be validated even without an observer"
        );
    }
}

#[tokio::test]
async fn native_progressive_output_event_validates_scope_before_observer() {
    let observer = Arc::new(RecordingOutputObserver::default());
    let adapter = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let (callback, _) = callback_for_channels(
        HashMap::from([(
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new("telegram:bot-b"))),
        )]),
        HashMap::from([(
            "telegram:bot-b".into(),
            adapter as Arc<dyn ProgressiveChannelTransport>,
        )]),
        Some(observer.clone() as Arc<dyn ProgressiveOutputObserver>),
    );
    for invalid in [
        output_event_request(Some("telegram:missing"), Some("chat-42"), Some("turn-14")),
        output_event_request(Some("telegram:bot-b"), None, Some("turn-14")),
        output_event_request(Some("telegram:bot-b"), Some("chat-42"), None),
    ] {
        assert!(
            callback.dispatch_controller_backend(invalid).await.is_err(),
            "invalid output event scope must be rejected before observation"
        );
    }
    assert!(observer.seen.lock().await.is_empty());
}

#[tokio::test]
async fn native_progressive_callbacks_stop_after_channel_session_cancellation() {
    let observer = Arc::new(RecordingOutputObserver::default());
    let adapter = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let (callback, _context) = callback_for_channels(
        HashMap::from([(
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new("telegram:bot-b"))),
        )]),
        HashMap::from([(
            "telegram:bot-b".into(),
            adapter as Arc<dyn ProgressiveChannelTransport>,
        )]),
        Some(observer.clone() as Arc<dyn ProgressiveOutputObserver>),
    );

    callback.cancellation.cancel();
    assert!(callback
        .dispatch_controller_backend(output_event_request(
            Some("telegram:bot-b"),
            Some("chat-42"),
            Some("turn-15"),
        ))
        .await
        .is_err());
    assert!(observer.seen.lock().await.is_empty());
}

#[tokio::test]
async fn native_progressive_callbacks_fail_closed_after_transport_owner_drops() {
    let adapter = Arc::new(RecordingChannel::new("telegram:bot-b"));
    let (callback, context) = callback_for_channels(
        HashMap::from([(
            "telegram:bot-b".into(),
            Arc::new(ChannelHandle::test(TestChannel::new("telegram:bot-b"))),
        )]),
        HashMap::from([(
            "telegram:bot-b".into(),
            adapter.clone() as Arc<dyn ProgressiveChannelTransport>,
        )]),
        None,
    );
    assert!(matches!(
        callback
            .dispatch_controller_backend(output_event_request(
                Some("telegram:bot-b"),
                Some("chat-42"),
                Some("turn-16"),
            ))
            .await,
        Ok(ChannelBackendResult::ProgressiveTransport(result))
            if matches!(*result, ProgressiveTransportResult::Acknowledged)
    ));
    drop(context);
    drop(adapter);

    assert!(callback
        .dispatch_controller_backend(output_event_request(
            Some("telegram:bot-b"),
            Some("chat-42"),
            Some("turn-16"),
        ))
        .await
        .is_err());
}

#[path = "callback_inbound_tests.rs"]
mod inbound_tests;
