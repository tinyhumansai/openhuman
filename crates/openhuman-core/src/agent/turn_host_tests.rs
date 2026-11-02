use super::*;

#[test]
fn workspace_binding_clears_a_previous_turns_workspace() {
    let mut session = session_with_workspace("/previous/thread");
    assert_eq!(
        session.live_workspace_descriptor().unwrap().root,
        std::path::PathBuf::from("/previous/thread")
    );

    bind_workspace(&mut session, WorkspaceBinding::Clear);

    assert_eq!(session.live_workspace_descriptor(), None);
}

#[test]
fn workspace_binding_can_preserve_or_replace_the_session_workspace() {
    let previous = tinytools::WorkspaceDescriptor::new("/configured");
    let requested = tinytools::WorkspaceDescriptor::new("/thread");
    let mut session = session_with_workspace("/configured");

    bind_workspace(&mut session, WorkspaceBinding::Preserve);
    assert_eq!(session.live_workspace_descriptor(), Some(previous));

    bind_workspace(&mut session, WorkspaceBinding::Set(requested.clone()));
    assert_eq!(session.live_workspace_descriptor(), Some(requested));
}

fn session_with_workspace(path: &str) -> OpenHumanSessionHost {
    let root = tempfile::tempdir().expect("workspace tempdir");
    let model = std::sync::Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "ok",
    ]));
    let chat_model: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>> = model;
    crate::agent::SessionHostBuilder::new()
        .chat_model(chat_model)
        .model_name("test-model".to_owned())
        .tools(Vec::new())
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .workspace_descriptor(Some(tinytools::WorkspaceDescriptor::new(path)))
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session host")
}
