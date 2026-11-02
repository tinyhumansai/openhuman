use super::*;

#[tokio::test]
async fn debug_prompt_matches_the_system_prompt_sent_on_a_cold_turn() {
    let _ = crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "ok",
    ]));
    let model_for_host: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model.clone();
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model_for_host)
        .model_name("test/session-fixture".to_string())
        .tools(crate::agent::goals::goal_tools(action_dir.path()))
        .action_dir(action_dir.path().to_path_buf())
        .workspace_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");
    host.set_thread_id(None::<&str>);
    host.ensure_runtime_session().expect("runtime session");
    host.refresh_prompt_surface().await;

    let dumped = host
        .render_current_turn_prompt()
        .expect("debug prompt surface");
    host.run_single("hello").await.expect("scripted turn");
    let request = model.requests().into_iter().next().expect("model request");
    let sent = request
        .messages
        .iter()
        .filter_map(|message| match message {
            tinyinference_llm::message::Message::System(system) => Some(
                system
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        tinyinference_llm::message::ContentBlock::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect::<String>();

    for goal_tool in ["goal_get", "goal_set", "goal_complete"] {
        assert!(
            !sent.contains(goal_tool),
            "{goal_tool} leaked into the actual system prompt"
        );
        assert!(
            !request.tools.iter().any(|tool| tool.name == goal_tool),
            "{goal_tool} leaked into the actual tool declarations"
        );
    }
    assert_eq!(
        dumped, sent,
        "debug dump must equal the real cold-turn system prompt"
    );
}
