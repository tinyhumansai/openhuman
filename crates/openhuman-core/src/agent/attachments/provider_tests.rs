use super::*;
use std::sync::Mutex;
#[derive(Default)]
pub(super) struct Probe {
    pub(super) requests: Mutex<Vec<ModelRequest>>,
    profile: ModelProfile,
}
#[async_trait]
impl ChatModel<()> for Probe {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }
    fn supports_input(&self, m: InputModality, t: &str, s: InputSource) -> bool {
        m == InputModality::Image
            && matches!(t, "image/png" | "image/jpeg")
            && s == InputSource::Base64
    }
    async fn invoke(
        &self,
        _: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.requests.lock().unwrap().push(request);
        Err(tinyinference_llm::Error::Model("probe".into()))
    }
}
#[tokio::test]
async fn local_native_resolution_changes_only_ephemeral_request() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("sample.png"), b"original-png")
        .await
        .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    config.workspace_dir = temp.path().join("internal");
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "sample.png".into(),
                mime_type: Some("image/png".into()),
            })],
        },
    )]);
    let durable = request.clone();
    assert!(wrapper.invoke(&(), request).await.is_err());
    assert!(serde_json::to_string(&durable)
        .unwrap()
        .contains("sample.png"));
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(serde_json::to_string(&captured[0])
        .unwrap()
        .contains("data:image/png;base64,"));
    assert_eq!(
        tokio::fs::read(temp.path().join("sample.png"))
            .await
            .unwrap(),
        b"original-png"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn blocked_attachment_fails_before_model_call() {
    let mut config = Config::default();
    config.multimodal_files.max_files = 0;
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "../secret.png".into(),
                mime_type: Some("image/png".into()),
            })],
        },
    )]);
    assert!(wrapper.invoke(&(), request).await.is_err());
    assert!(probe.requests.lock().unwrap().is_empty());
}
#[test]
fn unknown_model_does_not_inherit_transport_vision() {
    let model = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(Probe::default()),
        Arc::new(Config::default()),
        "unknown-model",
        "cloud",
    );
    assert!(!model.profile().unwrap().modalities.image_in);
}

#[tokio::test]
async fn persisted_image_without_mime_uses_magic_over_extension() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("renamed.png"), [0xff, 0xd8, 0xff, 0xe0])
        .await
        .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "renamed.png".into(),
                mime_type: None,
            })],
        },
    )]);
    assert!(wrapper.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(serde_json::to_string(&captured[0])
        .unwrap()
        .contains("data:image/jpeg;base64,"));
}

#[tokio::test]
async fn too_many_typed_images_reject_before_read_or_inference() {
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(Config::default()),
        "gpt-4o",
        "openai",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: (0..5)
                .map(|_| {
                    ContentBlock::Image(ImageRef {
                        url: "missing.png".into(),
                        mime_type: None,
                    })
                })
                .collect(),
        },
    )]);
    let error = wrapper.invoke(&(), request).await.unwrap_err();
    assert!(error.to_string().contains("attachment count"));
    assert!(probe.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn external_channel_cannot_rehydrate_a_local_image() {
    use crate::agent::turn_origin::AgentTurnOrigin;
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(Config::default()),
        "gpt-4o",
        "openai",
    );
    let mut request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "missing.png".into(),
                mime_type: None,
            })],
        },
    )]);
    let origin = AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "telegram".into(),
        sender: None,
        reply_target: "chat".into(),
        message_id: "message".into(),
        history_key: None,
    };
    let mut context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    context.origin = Some(origin);
    super::super::attach_request_scope(&mut request, &context);
    let error = wrapper.invoke(&(), request).await.unwrap_err();
    assert!(error.to_string().contains("external channel"));
    assert!(probe.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_video_retains_path_as_metadata_without_specialist() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("clip.mp4"), b"video-original")
        .await
        .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "unknown",
        "custom",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Video(MediaRef::Path {
                path: "clip.mp4".into(),
                media_type: Some("video/mp4".into()),
            })],
        },
    )]);
    assert!(wrapper.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    let text = captured[0].messages[0].text();
    assert!(text.contains("clip.mp4"));
    assert!(text.contains("14 bytes"));
    assert!(!serde_json::to_string(&captured[0])
        .unwrap()
        .contains("base64"));
}

#[tokio::test]
async fn derivative_cache_survives_model_recreation_and_invalidates_changed_content() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("uploads/thread/id/original.txt");
    tokio::fs::create_dir_all(original.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&original, b"original").await.unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let config = Arc::new(config);
    let make = || AttachmentModel {
        inner: Arc::new(Probe::default()),
        config: config.clone(),
        profile: ModelProfile::default(),
        fallback_cache: Mutex::new(Default::default()),
        core: None,
    };
    let first = make();
    let scope = super::super::AttachmentAccessScope::default();
    let key = first.fallback_key("uploads/thread/id/original.txt", "text/plain", b"original");
    first
        .save_fallback(
            "uploads/thread/id/original.txt",
            &key,
            "derived readout",
            &scope,
        )
        .await;
    let resumed = make();
    assert_eq!(
        resumed
            .cached_fallback("uploads/thread/id/original.txt", &key, &scope)
            .await
            .as_deref(),
        Some("derived readout")
    );
    let changed = first.fallback_key("uploads/thread/id/original.txt", "text/plain", b"modified");
    assert!(resumed
        .cached_fallback("uploads/thread/id/original.txt", &changed, &scope)
        .await
        .is_none());
    assert_eq!(tokio::fs::read(original).await.unwrap(), b"original");
    assert!(tokio::fs::read_dir(temp.path().join("uploads/thread/id"))
        .await
        .unwrap()
        .next_entry()
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn cache_is_not_written_beside_unmanaged_local_files() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("original.txt"), b"original")
        .await
        .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let model = AttachmentModel {
        inner: Arc::new(Probe::default()),
        config: Arc::new(config),
        profile: ModelProfile::default(),
        fallback_cache: Mutex::new(Default::default()),
        core: None,
    };
    model
        .save_fallback(
            "original.txt",
            "key",
            "derived",
            &super::super::AttachmentAccessScope::default(),
        )
        .await;
    assert!(!temp.path().join(".openhuman-intake.txt").exists());
}

#[test]
fn endpoint_discovery_denial_overrides_offline_vision_facts() {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use tinyinference_llm::model::discover::{model_limits_cache, LimitSource, ModelLimits};
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = temp.path().join("config.toml");
    config.workspace_dir = temp.path().join("workspace");
    config.cloud_providers = vec![CloudProviderCreds {
        id: "p_attachment_probe".into(),
        slug: "attachment-probe".into(),
        label: "test".into(),
        endpoint: "http://127.0.0.1:19991/v1".into(),
        auth_style: AuthStyle::None,
        ..Default::default()
    }];
    let request = crate::inference::provider::factory::model_limits_request(
        "chat",
        "attachment-probe:gpt-4o",
        "gpt-4o",
        &config,
    )
    .unwrap();
    model_limits_cache().insert_discovered_variant(
        &request.endpoint,
        &request.model,
        &request.cache_variant(),
        Some(ModelLimits {
            context_window: None,
            max_output_tokens: None,
            input_modalities: Some(vec!["text".into()]),
            source: LimitSource::ProviderListing,
        }),
    );
    let model = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(Probe::default()),
        Arc::new(config),
        "gpt-4o",
        "attachment-probe",
    );
    assert!(!model.profile().unwrap().modalities.image_in);
}

#[tokio::test]
async fn legacy_inline_source_is_bounded_and_malformed_base64_rejects_before_primary() {
    let probe = Arc::new(Probe::default());
    let mut config = Config::default();
    config.multimodal.max_image_size_mb = 1;
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    for source in [
        "data:image/png;base64,%%%".to_owned(),
        format!("data:image/png;base64,{}", "A".repeat(2 * 1024 * 1024)),
    ] {
        let request = ModelRequest::new(vec![Message::User(
            tinyinference_llm::message::UserMessage {
                content: vec![ContentBlock::Image(ImageRef {
                    url: source,
                    mime_type: Some("image/png".into()),
                })],
            },
        )]);
        assert!(wrapper.invoke(&(), request).await.is_err());
    }
    assert!(probe.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disabled_remote_replay_is_rejected_before_inference() {
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(Config::default()),
        "unknown",
        "custom",
    );
    for block in [
        ContentBlock::Image(ImageRef {
            url: "https://example.invalid/image.png".into(),
            mime_type: Some("image/png".into()),
        }),
        ContentBlock::Document(MediaRef::Url {
            url: "https://example.invalid/file.pdf".into(),
            media_type: Some("application/pdf".into()),
        }),
    ] {
        let request = ModelRequest::new(vec![Message::User(
            tinyinference_llm::message::UserMessage {
                content: vec![block],
            },
        )]);
        let error = wrapper.invoke(&(), request).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("remote attachment fetch is disabled"));
    }
    assert!(probe.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inline_native_replay_materializes_original_and_keeps_durable_message_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    let bytes = b"\x89PNG\r\n\x1a\noriginal";
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
                mime_type: None,
            })],
        },
    )]);
    let durable = request.clone();
    assert!(wrapper.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let text = captured[0].messages[0].text();
    let path = text
        .split("workspace path: ")
        .nth(1)
        .unwrap()
        .trim_end_matches(']');
    assert_eq!(
        tokio::fs::read(temp.path().join(path)).await.unwrap(),
        bytes
    );
    assert_eq!(durable.messages[0].text(), "");
}

#[tokio::test]
async fn derivative_cache_cannot_overwrite_original_named_like_cache() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let source = format!(
        "data:text/plain;name=.openhuman-intake.txt;base64,{}",
        STANDARD.encode(b"original")
    );
    let staged = super::super::stage(
        &format!("[FILE:{source}]"),
        "thread",
        &config,
        &super::super::AttachmentAccessScope::default(),
    )
    .await
    .unwrap();
    let (_, files) = super::super::parse(&staged);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, ".openhuman-intake.txt");
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config.clone()),
        "unknown",
        "custom",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Document(MediaRef::Path {
                path: files[0].path.clone(),
                media_type: Some("text/plain".into()),
            })],
        },
    )]);
    assert!(wrapper.invoke(&(), request).await.is_err());
    assert_eq!(
        tokio::fs::read(temp.path().join(&files[0].path))
            .await
            .unwrap(),
        b"original"
    );
}

struct InlineVision {
    profile: ModelProfile,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl ChatModel<()> for InlineVision {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }
    fn supports_input(&self, m: InputModality, mime: &str, source: InputSource) -> bool {
        m == InputModality::Image && mime == "image/png" && source == InputSource::Base64
    }
    async fn invoke(
        &self,
        _: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        assert!(
            matches!(&request.messages[0],Message::User(user) if user.content.iter().any(|b|matches!(b,ContentBlock::Image(_))))
        );
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ModelResponse::assistant("specialist image readout"))
    }
}

#[tokio::test]
async fn unknown_model_legacy_inline_replay_uses_specialist_once_across_model_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let config = Arc::new(config);
    let mut profile = ModelProfile::default();
    profile.provider = Some("injected".into());
    profile.modalities.image_in = true;
    let specialist = Arc::new(InlineVision {
        profile,
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let _guard = crate::inference::provider::factory::test_provider_override::install_model(
        specialist.clone(),
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(b"\x89PNG\r\n\x1a\noriginal")
                ),
                mime_type: None,
            })],
        },
    )]);
    for _ in 0..2 {
        let main = Arc::new(Probe::default());
        let wrapper = wrap(
            &crate::core::runtime::DispatchCtx::host_default(),
            main.clone(),
            config.clone(),
            "unknown-model",
            "custom",
        );
        assert!(wrapper.invoke(&(), request.clone()).await.is_err());
        let captured = main.requests.lock().unwrap().clone();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].messages[0]
            .text()
            .contains("specialist image readout"));
        assert!(!serde_json::to_string(&captured[0])
            .unwrap()
            .contains("base64"));
    }
    assert_eq!(
        specialist.calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn legacy_audio_base64_unsupported_transport_becomes_workspace_reference() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let main = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        main.clone(),
        Arc::new(config),
        "unknown",
        "custom",
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Audio(MediaRef::Base64 {
                data: STANDARD.encode(b"original-audio"),
                media_type: "audio/wav".into(),
            })],
        },
    )]);
    assert!(wrapper.invoke(&(), request).await.is_err());
    let captured = main.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let text = captured[0].messages[0].text();
    assert!(text.contains("audio/wav"));
    assert!(text.contains("workspace path: uploads/legacy-media/"));
    assert!(!serde_json::to_string(&captured[0])
        .unwrap()
        .contains("base64"));
}

#[tokio::test]
async fn injected_turn_source_uses_bound_workspace_and_explicit_model_facts() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(
        temp.path().join("injected.png"),
        b"\x89PNG\r\n\x1a\noriginal",
    )
    .await
    .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let mut probe = Probe::default();
    probe.profile.provider = Some("injected".into());
    probe.profile.modalities.image_in = true;
    let probe = Arc::new(probe);
    let source = crate::agent::tinyagents::TurnModelSource::from_model(probe.clone())
        .with_attachment_config(Arc::new(config));
    let models = source
        .build("explicit-injected-model", 0.0, None, None)
        .unwrap();
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "injected.png".into(),
                mime_type: None,
            })],
        },
    )]);
    assert!(models.primary.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(serde_json::to_string(&captured[0])
        .unwrap()
        .contains("data:image/png;base64,"));
}

#[tokio::test]
async fn configured_injected_builder_stages_and_resolves_in_explicit_acting_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let acting = temp.path().join("override-action");
    let mut config = Config::default();
    config.workspace_dir = temp.path().join("internal");
    config.action_dir = temp.path().join("configured-action");
    config.config_path = temp.path().join("config.toml");
    let mut probe = Probe::default();
    probe.profile.provider = Some("injected".into());
    probe.profile.modalities.image_in = true;
    let probe = Arc::new(probe);
    let agent = crate::agent::SessionHostBuilder::new()
        .chat_model_with_config(probe.clone(), Arc::new(config.clone()))
        .action_dir(acting.clone())
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .unwrap();
    let bound = agent.runtime_config().unwrap();
    assert_eq!(bound.action_dir, acting);
    assert_eq!(bound.workspace_dir, config.workspace_dir);
    assert_eq!(bound.config_path, config.config_path);
    let bytes = b"\x89PNG\r\n\x1a\noriginal";
    let staged = super::super::stage_turn(
        &format!("[IMAGE:data:image/png;base64,{}]", STANDARD.encode(bytes)),
        Some(&bound),
        None,
        Some("builder-test"),
        None,
    )
    .await
    .unwrap();
    let (_, attachments) = super::super::parse(&staged);
    assert_eq!(attachments.len(), 1);
    assert_eq!(
        tokio::fs::read(acting.join(&attachments[0].path))
            .await
            .unwrap(),
        bytes
    );
    let models = agent
        .turn_model_source()
        .build("explicit-injected", 0.0, None, None)
        .unwrap();
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: attachments[0].path.clone(),
                mime_type: None,
            })],
        },
    )]);
    assert!(models.primary.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(serde_json::to_string(&captured[0])
        .unwrap()
        .contains("data:image/png;base64,"));
}

#[test]
fn bare_injected_builder_does_not_load_operator_config() {
    let agent = crate::agent::SessionHostBuilder::new()
        .chat_model(Arc::new(Probe::default()))
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .unwrap();
    assert!(agent.runtime_config().is_none());
}

#[cfg(feature = "documents")]
#[tokio::test]
async fn pdf_attachment_degrades_to_a_note_when_documents_component_is_off() {
    let mut config = Config::default();
    config.components.documents = false;
    let model = AttachmentModel {
        inner: Arc::new(Probe::default()),
        config: Arc::new(config),
        profile: ModelProfile::default(),
        fallback_cache: Mutex::new(Default::default()),
        core: None,
    };
    let (text, cacheable) = model
        .fallback(
            InputModality::Document,
            "uploads/t/id/brief.pdf",
            "application/pdf",
            b"%PDF-1.4",
        )
        .await
        .unwrap();
    assert!(text.contains("Document extraction is disabled"), "{text}");
    assert!(text.contains("brief.pdf"));
    assert!(!cacheable);
}
