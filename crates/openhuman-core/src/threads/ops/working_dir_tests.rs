use super::*;

#[tokio::test]
async fn empty_request_uses_the_global_folder() {
    assert_eq!(validate_working_dir(None).await.unwrap(), None);
    assert_eq!(validate_working_dir(Some("   ")).await.unwrap(), None);
}

#[tokio::test]
async fn existing_absolute_directory_is_canonicalized() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("project");
    std::fs::create_dir(&nested).unwrap();
    let raw = format!("{}/./", nested.display());
    let validated = validate_working_dir(Some(&raw)).await.unwrap().unwrap();
    assert_eq!(PathBuf::from(validated), nested.canonicalize().unwrap());
}

#[tokio::test]
async fn relative_missing_file_and_forbidden_paths_are_rejected() {
    assert!(validate_working_dir(Some("relative/dir")).await.is_err());

    let temp = tempfile::tempdir().unwrap();
    assert!(
        validate_working_dir(Some(&temp.path().join("missing").display().to_string()))
            .await
            .is_err()
    );

    let file = temp.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(validate_working_dir(Some(&file.display().to_string()))
        .await
        .is_err());

    let ssh = temp.path().join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    assert!(validate_working_dir(Some(&ssh.display().to_string()))
        .await
        .is_err());
}

fn thread(id: &str, working_dir: Option<String>) -> conversations::CreateConversationThread {
    conversations::CreateConversationThread {
        id: id.into(),
        title: id.into(),
        created_at: "2026-10-23T00:00:00Z".into(),
        parent_thread_id: None,
        labels: None,
        personality_id: None,
        working_dir,
    }
}

#[tokio::test]
async fn a_bound_folder_that_disappears_fails_closed() {
    // The thread must not silently fall back to the global folder.
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("internal");
    let folder = temp.path().join("project");
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let ctx = DispatchCtx::ambient();
    conversations::blocking::ensure_thread(
        ctx.clone(),
        workspace.clone(),
        thread("bound", Some(folder.to_string_lossy().into_owned())),
    )
    .await
    .unwrap();
    assert_eq!(
        thread_working_dir(&ctx, workspace.clone(), "bound")
            .await
            .unwrap(),
        Some(folder.clone())
    );

    std::fs::remove_dir(&folder).unwrap();
    let error = thread_working_dir(&ctx, workspace, "bound")
        .await
        .expect_err("a vanished bound folder");
    assert!(error.contains("unusable"), "{error}");
}

#[tokio::test]
async fn the_working_folder_is_fixed_after_the_first_message() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let home = tempfile::tempdir().unwrap();
    // Cleared on drop, even if an assertion below panics.
    let _workspace = crate::config::test_env::EnvVarGuard::workspace_unlocked(home.path());
    let ctx = DispatchCtx::ambient();
    let workspace = workspace_dir().await.unwrap();
    conversations::blocking::ensure_thread(ctx.clone(), workspace.clone(), thread("t", None))
        .await
        .unwrap();
    let folder = home.path().join("project");
    std::fs::create_dir(&folder).unwrap();
    let request = |dir: &Path| UpdateConversationThreadWorkingDirRequest {
        thread_id: "t".into(),
        action_dir: dir.display().to_string(),
    };
    // Before any message the folder can be bound.
    thread_update_working_dir(&ctx, request(&folder))
        .await
        .unwrap();

    conversations::blocking::append_message(
        ctx.clone(),
        workspace,
        "t".into(),
        conversations::ConversationMessage {
            id: "m1".into(),
            content: "hi".into(),
            message_type: "text".into(),
            extra_metadata: serde_json::json!({}),
            sender: "user".into(),
            created_at: "2026-10-23T00:00:01Z".into(),
        },
    )
    .await
    .unwrap();
    let error = thread_update_working_dir(&ctx, request(home.path()))
        .await
        .map(|_| ())
        .expect_err("rebinding after the first message");
    assert!(
        error
            .to_string()
            .contains("before the conversation's first message"),
        "{error}"
    );
}
