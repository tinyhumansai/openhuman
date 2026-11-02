use super::*;

#[tokio::test]
async fn a_binding_no_agent_answers_is_refused_not_sent_to_the_orchestrator() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;
    let tmp = tempfile::tempdir().unwrap();
    let channel = Arc::new(crate::channels::runtime::TestChannel::new("telegram"));
    let ctx = context(
        Arc::clone(&channel),
        config(&tmp, Some("nobody-registered")),
    );

    process_channel_message(ctx, message("hello")).await;

    let sent = channel
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("not available"), "{sent:?}");
}

#[test]
fn an_unbound_channel_still_runs_the_orchestrator() {
    on_agent_runtime(an_unbound_channel_runs_the_orchestrator);
}

async fn an_unbound_channel_runs_the_orchestrator() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .unwrap();
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer("orchestrator answered")))
        .mount(&provider)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config(&tmp, None);
    config.default_model = Some("fixture".into());
    let route = crate::config::schema::EphemeralRoute::from_params(
        Some(format!("{}/v1", provider.uri())),
        Some("sk-fixture".into()),
    )
    .unwrap();
    crate::config::schema::ephemeral_route::apply(&mut config, route);
    config
        .agent
        .channel_agents
        .insert("discord".into(), "some-agent".into());
    let channel = Arc::new(crate::channels::runtime::TestChannel::new("telegram"));

    let mut ctx = context(Arc::clone(&channel), config.clone());
    Arc::get_mut(&mut ctx)
        .expect("the channel context is shared only by this test")
        .default_provider = Arc::new(crate::config::schema::EPHEMERAL_ROUTE_SLUG.to_string());
    Arc::get_mut(&mut ctx)
        .expect("the channel context remains uniquely owned")
        .model = Arc::new("fixture".into());
    let core = Arc::new(CoreContext::for_test_with_config(DomainSet::full(), config));
    Arc::get_mut(&mut ctx)
        .expect("the channel context is shared only by this test")
        .core = Some(Arc::downgrade(&core));
    process_channel_message(ctx, message("hello")).await;

    assert_eq!(
        channel
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        vec!["orchestrator answered".to_string()]
    );
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the configured orchestrator calls inference"
    );
    let request: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        request["messages"]
            .as_array()
            .is_some_and(|messages| messages.iter().any(|message| {
                message["role"] == "user"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("hello"))
            })),
        "the unbound channel message reaches the orchestrator model: {request:#}"
    );
}
