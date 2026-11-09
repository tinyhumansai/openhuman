use super::tests::Probe;
use super::*;

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
        .await
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
        .model_name("fixture/injected".to_string())
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
        .await
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
        .model_name("fixture/injected".to_string())
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .unwrap();
    assert!(agent.runtime_config().is_none());
}
