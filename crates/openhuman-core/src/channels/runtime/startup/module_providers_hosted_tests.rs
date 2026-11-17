use super::*;

#[tokio::test]
async fn hosted_target_projects_to_the_canonical_thread_and_keeps_one_gateway_terminal() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let _env_lock = EnvVarGuard::locked_async().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    let request_id = "relay-target-projection-request";
    let client_id = "relay-target-projection-client";
    let thread_id = "channel:telegram/sender-7/chat-42";
    let mut web_events = crate::web_chat::subscribe_web_channel_events();
    let relay = Arc::new(crate::channels::providers::relay::RelayChannel::new(
        "telegram",
        client_id,
        thread_id,
        request_id,
        "provider-message-9",
    ));
    let channel = tinychannels_bus::ChannelRef {
        id: "telegram".into(),
        account_id: Some("trusted-account-7".into()),
    };
    let conversation = tinychannels_bus::ConversationRef {
        kind: tinychannels_bus::ConversationKind::Group,
        id: "chat-42".into(),
        thread_id: Some("topic-3".into()),
        topic_id: Some("topic-3".into()),
        ..Default::default()
    };
    let config = Config::default();
    let mut overlay = ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none())
        .session_agent("relay-target-agent");
    overlay.profile = Some("relay-target-profile".into());
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay));
    let target = crate::channels::runtime::dispatch::hosted_relay_target(
        &ctx,
        &config,
        Arc::clone(&relay),
        channel,
        conversation,
        client_id.into(),
        thread_id.into(),
        request_id.into(),
    );
    let reply = target
        .open("relay-target-projection-turn")
        .await
        .expect("hosted target opens the compiled progressive module");
    let delivery = reply
        .finish("projected hosted reply")
        .await
        .expect("compiled relay output is delivered");
    assert!(crate::channels::runtime::dispatch::delivery_succeeded(
        &delivery
    ));
    assert_eq!(relay.sent(), vec!["projected hosted reply"]);

    let mut projected = None;
    let mut gateway_terminal = None;
    let both_events = tokio::time::timeout(Duration::from_secs(5), async {
        while projected.is_none() || gateway_terminal.is_none() {
            let event = web_events.recv().await.expect("web event bus");
            if event.context().request_id != request_id {
                continue;
            }
            match event.event_name() {
                "chat_done" => projected = Some(event),
                crate::channels::providers::relay::CHANNEL_OUTBOUND_EVENT => {
                    gateway_terminal = Some(event)
                }
                _ => panic!("unexpected relayed output event: {}", event.event_name()),
            }
        }
    })
    .await;
    if both_events.is_err() {
        assert!(
            gateway_terminal.is_some(),
            "the scoped provider terminal should still arrive independently"
        );
        panic!("provider terminal arrived, but the web projector did not emit chat_done");
    }

    let projected = projected.expect("canonical projector output");
    let projected_wire = serde_json::to_value(&projected).expect("web renderer wire");
    assert_eq!(projected.context().client_id, client_id);
    assert_eq!(projected.context().thread_id, thread_id);
    assert_eq!(projected.context().seq, Some(1));
    assert!(projected.belongs_to("relay-target-agent"));
    assert!(projected.belongs_to_profile("relay-target-profile"));
    assert_eq!(projected_wire["full_response"], "projected hosted reply");

    let gateway_terminal = gateway_terminal.expect("legacy SaaS terminal");
    let terminal_wire = serde_json::to_value(&gateway_terminal).expect("gateway wire");
    assert_eq!(gateway_terminal.context().client_id, client_id);
    assert_eq!(gateway_terminal.context().thread_id, thread_id);
    assert!(gateway_terminal.belongs_to("relay-target-agent"));
    assert!(gateway_terminal.belongs_to_profile("relay-target-profile"));
    assert_eq!(terminal_wire["structured"]["kind"], "channel_outbound");
    assert_eq!(relay.sent().len(), 1, "one provider terminal is published");

    while let Ok(event) = web_events.try_recv() {
        assert_ne!(
            event.context().request_id,
            request_id,
            "no duplicate output"
        );
    }
    reply
        .close()
        .await
        .expect("hosted projection runtime closes");
}

#[tokio::test]
async fn hosted_target_projects_typed_media_and_voice_to_web_and_gateway_once() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinychannels_bus::{ContentPart, MediaKind, MediaReference, VoiceNote};

    let _env_lock = EnvVarGuard::locked_async().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    let request_id = "relay-typed-output-request";
    let client_id = "relay-typed-output-client";
    let thread_id = "channel:telegram/sender-7/chat-42";
    let mut web_events = crate::web_chat::subscribe_web_channel_events();
    let relay = Arc::new(crate::channels::providers::relay::RelayChannel::new(
        "telegram",
        client_id,
        thread_id,
        request_id,
        "provider-message-9",
    ));
    let channel = tinychannels_bus::ChannelRef {
        id: "telegram".into(),
        account_id: Some("trusted-account-7".into()),
    };
    let conversation = tinychannels_bus::ConversationRef {
        kind: tinychannels_bus::ConversationKind::Group,
        id: "chat-42".into(),
        thread_id: Some("topic-3".into()),
        topic_id: Some("topic-3".into()),
        ..Default::default()
    };
    let config = Config::default();
    let mut overlay = ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none())
        .session_agent("relay-typed-output-agent");
    overlay.profile = Some("relay-typed-output-profile".into());
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay));
    let target = crate::channels::runtime::dispatch::hosted_relay_target(
        &ctx,
        &config,
        Arc::clone(&relay),
        channel.clone(),
        conversation.clone(),
        client_id.into(),
        thread_id.into(),
        request_id.into(),
    );
    let reply = target
        .open("relay-typed-output-turn")
        .await
        .expect("hosted target opens the compiled progressive module");

    let image_ref = MediaReference {
        opaque_id: Some("media:tenant7/thread42/image1".into()),
        content_type: Some("image/png".into()),
        kind: MediaKind::Image,
        ..Default::default()
    };
    let voice_ref = MediaReference {
        opaque_id: Some("media:tenant7/thread42/voice1".into()),
        content_type: Some("audio/ogg".into()),
        kind: MediaKind::Audio,
        ..Default::default()
    };
    let media_result = reply
        .feed(vec![
            ChannelOutputEvent::Media {
                text: Some("Generated image".into()),
                parts: vec![ContentPart::Image {
                    media: image_ref.clone(),
                    caption: None,
                    alt_text: Some("generated scene".into()),
                }],
            },
            ChannelOutputEvent::VoiceNote {
                text: None,
                voice: VoiceNote {
                    media: voice_ref.clone(),
                    codec: Some("opus".into()),
                    duration_ms: Some(900),
                    ..Default::default()
                },
            },
        ])
        .await
        .expect("typed media and voice are accepted by the hosted progressive target");
    assert!(crate::channels::runtime::dispatch::delivery_succeeded(
        &media_result
    ));
    let final_result = reply
        .finish("Final hosted reply")
        .await
        .expect("the provider terminal is delivered");
    assert!(crate::channels::runtime::dispatch::delivery_succeeded(
        &final_result
    ));

    let mut projected: Vec<crate::web_chat::ScopedWebEvent> = Vec::new();
    let mut gateway_terminals = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while projected
            .iter()
            .filter(|e: &&crate::web_chat::ScopedWebEvent| e.event_name() == "chat_message")
            .count()
            < 2
            || projected
                .iter()
                .filter(|e: &&crate::web_chat::ScopedWebEvent| e.event_name() == "chat_done")
                .count()
                < 1
            || gateway_terminals.len() < 3
        {
            let event = web_events.recv().await.expect("web event bus");
            if event.context().request_id != request_id {
                continue;
            }
            match event.event_name() {
                "chat_message" | "chat_done" => projected.push(event),
                crate::channels::providers::relay::CHANNEL_OUTBOUND_EVENT => {
                    gateway_terminals.push(event)
                }
                name => panic!("unexpected typed relay output event: {name}"),
            }
        }
    })
    .await
    .expect("both typed projections and all provider sends arrive");

    let image_projection = projected
        .iter()
        .find(|event| {
            serde_json::to_value(event)
                .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/image1"))
        })
        .expect("image part reaches the shared web projection");
    let image_projection_wire = serde_json::to_value(image_projection).expect("image wire");
    assert_eq!(image_projection.context().client_id, client_id);
    assert_eq!(image_projection.context().thread_id, thread_id);
    assert!(image_projection.belongs_to("relay-typed-output-agent"));
    assert!(image_projection.belongs_to_profile("relay-typed-output-profile"));
    assert_eq!(
        image_projection_wire["parts"][0]["part"]["media"]["opaqueId"],
        "media:tenant7/thread42/image1"
    );

    let voice_projection = projected
        .iter()
        .find(|event| {
            serde_json::to_value(event)
                .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/voice1"))
        })
        .expect("voice note reaches the shared web projection");
    let voice_projection_wire = serde_json::to_value(voice_projection).expect("voice wire");
    assert_eq!(
        voice_projection_wire["parts"][0]["part"]["kind"],
        "voice_note"
    );
    assert_eq!(
        voice_projection_wire["parts"][0]["part"]["voiceNote"]["media"]["opaqueId"],
        "media:tenant7/thread42/voice1"
    );

    assert_eq!(
        projected
            .iter()
            .filter(|event| event.event_name() == "chat_done")
            .count(),
        1
    );
    assert_eq!(
        gateway_terminals.len(),
        3,
        "one provider terminal per typed send"
    );
    let image_gateway = gateway_terminals
        .iter()
        .find(|event| {
            serde_json::to_value(event)
                .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/image1"))
        })
        .expect("image part reaches the scoped provider terminal");
    let image_gateway_wire = serde_json::to_value(image_gateway).expect("gateway image wire");
    assert_eq!(image_gateway.context().client_id, client_id);
    assert_eq!(image_gateway.context().thread_id, thread_id);
    assert!(image_gateway.belongs_to("relay-typed-output-agent"));
    assert!(image_gateway.belongs_to_profile("relay-typed-output-profile"));
    assert!(image_gateway_wire["structured"]["parts"]
        .as_array()
        .expect("typed provider parts")
        .iter()
        .any(|part| {
            part["part"]["kind"] == "image"
                && part["part"]["media"]["opaqueId"] == "media:tenant7/thread42/image1"
        }));
    let voice_gateway = gateway_terminals
        .iter()
        .find(|event| {
            serde_json::to_value(event)
                .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/voice1"))
        })
        .expect("voice part reaches the scoped provider terminal");
    let voice_gateway_wire = serde_json::to_value(voice_gateway).expect("gateway voice wire");
    assert_eq!(voice_gateway.context().client_id, client_id);
    assert_eq!(voice_gateway.context().thread_id, thread_id);
    assert!(voice_gateway.belongs_to("relay-typed-output-agent"));
    assert!(voice_gateway.belongs_to_profile("relay-typed-output-profile"));
    assert!(voice_gateway_wire["structured"]["parts"]
        .as_array()
        .expect("typed voice provider parts")
        .iter()
        .any(|part| {
            part["kind"] == "voice"
                && part["voice"]["kind"] == "media"
                && part["voice"]["media"]["opaqueId"] == "media:tenant7/thread42/voice1"
        }));
    let expected_channel_ref = serde_json::to_value(&channel).expect("expected channel ref");
    let expected_conversation_ref =
        serde_json::to_value(&conversation).expect("expected conversation ref");
    assert_eq!(
        voice_gateway_wire["structured"]["channel_ref"],
        expected_channel_ref
    );
    assert_eq!(
        voice_gateway_wire["structured"]["conversation_ref"],
        expected_conversation_ref
    );
    assert_eq!(
        image_gateway_wire["structured"]["channel_ref"],
        expected_channel_ref
    );
    assert_eq!(
        image_gateway_wire["structured"]["conversation_ref"],
        expected_conversation_ref
    );
    assert_eq!(
        gateway_terminals
            .iter()
            .filter(|event| {
                serde_json::to_value(event)
                    .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/image1"))
            })
            .count(),
        1,
        "image delivery has one provider terminal"
    );
    assert!(
        image_gateway_wire["structured"]["parts"]
            .as_array()
            .expect("typed provider parts")
            .iter()
            .any(|part| {
                part["part"]["kind"] == "text" && part["part"]["text"] == "Generated image"
            }),
        "the media caption remains a typed text part in the provider terminal"
    );
    assert_eq!(
        gateway_terminals
            .iter()
            .filter(|event| {
                serde_json::to_value(event)
                    .is_ok_and(|wire| wire.to_string().contains("media:tenant7/thread42/voice1"))
            })
            .count(),
        1,
        "voice delivery has one provider terminal"
    );
    assert_eq!(
        gateway_terminals
            .iter()
            .filter(|event| {
                serde_json::to_value(event)
                    .is_ok_and(|wire| wire["full_response"] == "Final hosted reply")
            })
            .count(),
        1,
        "final text has one provider terminal"
    );
    assert_eq!(
        relay.sent().len(),
        3,
        "provider output is delivered once per send"
    );
    assert_eq!(
        projected.len(),
        3,
        "two media messages and one turn terminal"
    );

    reply
        .close()
        .await
        .expect("hosted projection runtime closes");
    while let Ok(event) = web_events.try_recv() {
        assert_ne!(
            event.context().request_id,
            request_id,
            "no duplicate typed projection or provider terminal"
        );
    }
}
