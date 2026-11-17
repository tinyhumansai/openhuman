use super::*;

fn persister(dir: &std::path::Path) -> TranscriptPersister {
    TranscriptPersister::new(
        DispatchCtx::ambient(),
        dir.to_path_buf(),
        "t1".into(),
        "s1".into(),
        "sarvam".into(),
    )
}

#[test]
fn builds_deterministic_messages_and_skips_blank_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = persister(dir.path());
    assert!(p.message(TranscriptRole::User, "   ").is_none());
    let user = p
        .message(TranscriptRole::User, " what time is it ")
        .unwrap();
    assert_eq!(user.id, "voice-s1-1-user");
    assert_eq!(user.sender, "user");
    assert_eq!(user.content, "what time is it");
    assert_eq!(user.extra_metadata["source"], "voice");
    assert_eq!(user.extra_metadata["provider"], "sarvam");
    let agent = p.message(TranscriptRole::Agent, "noon").unwrap();
    assert_eq!(agent.id, "voice-s1-2-agent");
    assert_eq!(agent.sender, "agent");
}

#[tokio::test]
async fn saves_into_the_thread_store() {
    let dir = tempfile::tempdir().unwrap();
    crate::threads::store::blocking::ensure_thread(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
        crate::threads::store::CreateConversationThread {
            id: "t1".into(),
            title: "Voice".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    let mut p = persister(dir.path());
    p.save(TranscriptRole::User, "hello").await;
    p.save(TranscriptRole::Agent, "hi there").await;
    p.save(TranscriptRole::Agent, "").await;
    let messages = crate::threads::store::blocking::get_messages(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
        "t1".into(),
    )
    .await
    .unwrap();
    let texts: Vec<_> = messages
        .iter()
        .map(|m| (m.sender.as_str(), m.content.as_str()))
        .collect();
    assert_eq!(texts, vec![("user", "hello"), ("agent", "hi there")]);
}

#[tokio::test]
async fn a_missing_thread_is_logged_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = persister(dir.path());
    // No thread created: the store refuses, the call still returns.
    p.save(TranscriptRole::User, "hello").await;
}
