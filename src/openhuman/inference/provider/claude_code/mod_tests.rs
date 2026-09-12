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
fn session_key_is_stable_for_same_conversation() {
    let a = vec![ChatMessage::user("hello world")];
    let b = vec![
        ChatMessage::user("hello world"),
        ChatMessage::assistant("hi"),
    ];
    assert_eq!(
        session_key_from_request(&a, Some("you are helpful")),
        session_key_from_request(&b, Some("you are helpful")),
    );
}

#[test]
fn session_key_diverges_for_different_first_user() {
    let a = vec![ChatMessage::user("alpha")];
    let b = vec![ChatMessage::user("beta")];
    assert_ne!(
        session_key_from_request(&a, None),
        session_key_from_request(&b, None),
    );
}

#[test]
fn session_key_diverges_for_services_sharing_one_thread() {
    // The reasoning/coding/agentic services all run the same thread through
    // this provider, and only the system prompt tells them apart. One key
    // between them is one `--resume` target between them, and each in turn
    // appends the same trailing user turn to it.
    let messages = vec![ChatMessage::user("list the files on your computer")];
    assert_ne!(
        session_key_from_request(&messages, Some("You are the reasoning service.")),
        session_key_from_request(&messages, Some("You are the coding service.")),
    );
}

#[test]
fn session_key_fields_cannot_be_shifted_into_each_other() {
    // Length-prefixed fields: moving a byte across the message/prompt
    // boundary has to change the key.
    let ab = vec![ChatMessage::user("ab")];
    let a = vec![ChatMessage::user("a")];
    assert_ne!(
        session_key_from_request(&ab, Some("c")),
        session_key_from_request(&a, Some("bc")),
    );
}
