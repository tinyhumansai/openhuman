use super::*;

/// A config whose state lives in `<root>/profile` (workspace and config.toml).
fn profile_config(root: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = root.join("profile").join("workspace");
    config.config_path = root.join("profile").join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[tokio::test]
async fn empty_request_uses_the_global_folder() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile_config(temp.path());
    assert_eq!(validate_working_dir(None, &config).await.unwrap(), None);
    assert_eq!(
        validate_working_dir(Some("   "), &config).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn existing_absolute_directory_is_canonicalized() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("project");
    std::fs::create_dir(&nested).unwrap();
    let raw = format!("{}/./", nested.display());
    let validated = validate_working_dir(Some(&raw), &profile_config(temp.path()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(PathBuf::from(validated), nested.canonicalize().unwrap());
}

#[tokio::test]
async fn relative_missing_file_and_forbidden_paths_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile_config(temp.path());
    assert!(validate_working_dir(Some("relative/dir"), &config)
        .await
        .is_err());

    assert!(validate_working_dir(
        Some(&temp.path().join("missing").display().to_string()),
        &config
    )
    .await
    .is_err());

    let file = temp.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(
        validate_working_dir(Some(&file.display().to_string()), &config)
            .await
            .is_err()
    );

    let ssh = temp.path().join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    assert!(
        validate_working_dir(Some(&ssh.display().to_string()), &config)
            .await
            .is_err()
    );
}

/// TS-E2E-021 on the thread path: a thread's working folder becomes the
/// agent's `action_dir`, so it must not sit inside or contain OpenHuman's
/// state. A folder beside that state stays allowed.
#[tokio::test]
async fn a_working_folder_inside_or_around_the_data_folder_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile_config(temp.path());
    let vault = config.workspace_dir.join("vault");
    std::fs::create_dir(&vault).unwrap();
    for dir in [
        config.workspace_dir.as_path(),
        vault.as_path(),
        temp.path().join("profile").as_path(),
        temp.path(),
    ] {
        let error = validate_working_dir(Some(&dir.display().to_string()), &config)
            .await
            .expect_err("a folder inside or around the data folder");
        assert!(error.contains("OpenHuman data folder"), "{dir:?}: {error}");
    }
    let beside = temp.path().join("profile").join("projects");
    std::fs::create_dir(&beside).unwrap();
    assert!(
        validate_working_dir(Some(&beside.display().to_string()), &config)
            .await
            .unwrap()
            .is_some()
    );
}

/// The thread store's workspace can differ from the context config's (single
/// user: the disk config vs the embedder config). A folder inside the store's
/// workspace is refused even when the context config doesn't name it.
#[tokio::test]
async fn a_working_folder_inside_the_thread_stores_workspace_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile_config(temp.path());
    let store = temp.path().join("store").join("workspace");
    let inside = store.join("threads");
    std::fs::create_dir_all(&inside).unwrap();
    let raw = inside.display().to_string();

    assert!(
        validate_working_dir(Some(&raw), &config).await.is_ok(),
        "the context config alone doesn't cover the store's workspace"
    );
    let error = validate_working_dir_for_store(Some(&raw), &config, &store)
        .await
        .expect_err("a folder inside the thread store's workspace");
    assert!(error.contains("OpenHuman data folder"), "{error}");
    assert!(
        validate_working_dir_for_store(Some(&raw), &config, &config.workspace_dir)
            .await
            .is_ok()
    );
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
    let config = profile_config(temp.path());
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
        thread_working_dir(&ctx, &config, workspace.clone(), "bound")
            .await
            .unwrap(),
        Some(folder.clone())
    );

    std::fs::remove_dir(&folder).unwrap();
    let error = thread_working_dir(&ctx, &config, workspace, "bound")
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
    let other = home.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let error = thread_update_working_dir(&ctx, request(&other))
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

/// The update RPC checks the folder against the loaded profile: with
/// `OPENHUMAN_WORKSPACE` set, the profile root holds config.toml and the
/// workspace, so binding it is refused and nothing is stored.
#[tokio::test]
async fn binding_the_profile_root_as_a_working_folder_is_refused() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let home = tempfile::tempdir().unwrap();
    let _workspace = crate::config::test_env::EnvVarGuard::workspace_unlocked(home.path());
    let ctx = DispatchCtx::ambient();
    let workspace = workspace_dir().await.unwrap();
    conversations::blocking::ensure_thread(ctx.clone(), workspace.clone(), thread("t", None))
        .await
        .unwrap();
    let error = thread_update_working_dir(
        &ctx,
        UpdateConversationThreadWorkingDirRequest {
            thread_id: "t".into(),
            action_dir: home.path().display().to_string(),
        },
    )
    .await
    .map(|_| ())
    .expect_err("the profile root holds OpenHuman's state");
    assert!(
        error.to_string().contains("OpenHuman data folder"),
        "{error}"
    );
    let stored = conversations::blocking::list_threads(ctx, workspace)
        .await
        .unwrap()
        .into_iter()
        .find(|thread| thread.id == "t")
        .unwrap();
    assert_eq!(stored.working_dir, None, "a refused folder is not stored");
}
