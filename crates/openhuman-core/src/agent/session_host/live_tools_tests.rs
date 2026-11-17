use super::*;

#[test]
fn exposes_durable_visible_tools_and_policy() {
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "ok",
    ]));
    let chat_model: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model;
    let mut host = crate::agent::SessionHostBuilder::new()
        .model_name("test-model".to_string())
        .chat_model(chat_model)
        .tools(Vec::new())
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session build");
    let surface = host.live_tool_surface();
    assert_eq!(surface.tool_sets.len(), 1);
    assert!(surface.allowed.is_empty());
    assert!(surface.tool_policy.is_some());
    assert!(!surface.has_thread);
    host.set_thread_id(Some("t1"));
    assert!(host.live_tool_surface().has_thread);
    let _ = host.live_workspace_descriptor();
}
