use super::*;

#[test]
fn chat_model_profile_advertises_native_streaming_tools() {
    let workspace = tempfile::tempdir().expect("workspace");
    let project = tempfile::tempdir().expect("project");
    let provider = ClaudeCodeProvider::new(
        "claude-sonnet-4-6",
        PathBuf::from("claude"),
        workspace.path().to_path_buf(),
        project.path().to_path_buf(),
        None,
    );

    let profile = provider.profile().expect("profile");
    assert_eq!(profile.provider.as_deref(), Some("claude-code"));
    assert_eq!(profile.model.as_deref(), Some("claude-sonnet-4-6"));
    assert!(profile.tool_calling);
    assert!(profile.parallel_tool_calls);
    assert!(profile.streaming);
    assert!(profile.streaming_tool_chunks);
}

#[test]
fn thread_key_is_stable_for_same_conversation() {
    let a = vec![ChatMessage::user("hello world")];
    let b = vec![
        ChatMessage::user("hello world"),
        ChatMessage::assistant("hi"),
    ];
    assert_eq!(thread_key_from_messages(&a), thread_key_from_messages(&b));
}

#[test]
fn thread_key_diverges_for_different_first_user() {
    let a = vec![ChatMessage::user("alpha")];
    let b = vec![ChatMessage::user("beta")];
    assert_ne!(thread_key_from_messages(&a), thread_key_from_messages(&b));
}
