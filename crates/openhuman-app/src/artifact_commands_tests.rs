use super::*;
use crate::test_env::env_lock;
use openhuman_rpc::embed::artifacts::FileRoots;

#[test]
fn sanitize_rejects_path_separators() {
    assert!(sanitize_filename("../etc/passwd").is_err());
    assert!(sanitize_filename("a\\b.pptx").is_err());
    assert!(sanitize_filename("a/b.pptx").is_err());
    assert!(sanitize_filename("").is_err());
    assert!(sanitize_filename(".").is_err());
    assert!(sanitize_filename("..").is_err());
    assert!(sanitize_filename("ok.pptx\0").is_err());
}

#[test]
fn sanitize_accepts_plain_names() {
    assert_eq!(
        sanitize_filename("Quarterly Update.pptx").unwrap(),
        "Quarterly Update.pptx"
    );
    assert_eq!(sanitize_filename("  trim me  ").unwrap(), "trim me");
}

async fn ready_artifact(workspace: &Path, files_dir: &Path) -> String {
    use openhuman_rpc::embed::artifacts::{create_artifact, finalize_artifact, ArtifactKind};
    let (meta, path) = create_artifact(
        workspace,
        files_dir,
        ArtifactKind::Presentation,
        "Deck",
        "pptx",
        None,
    )
    .await
    .unwrap();
    std::fs::write(&path, b"deck").unwrap();
    finalize_artifact(workspace, &meta.id, 4, None)
        .await
        .unwrap();
    meta.id
}

#[tokio::test]
async fn resolve_source_returns_the_file_in_the_files_folder() {
    // Core-process tests can install a process-global storage backend while
    // the artifact API resolves the record. Keep both operations on the same
    // stable host context, as in the other workspace-bound shell tests.
    let _env_lock = env_lock();
    let temp = tempfile::tempdir().unwrap();
    let files_dir = temp.path().join("Files");
    let id = ready_artifact(temp.path(), &files_dir).await;
    assert_eq!(
        resolve_source(temp.path(), &FileRoots::new(&files_dir), &id)
            .await
            .unwrap(),
        files_dir.join("deck.pptx")
    );
}

#[tokio::test]
async fn resolve_source_rejects_unknown_ids_and_paths() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        resolve_source(temp.path(), &FileRoots::new(temp.path()), "")
            .await
            .is_err()
    );
    assert!(resolve_source(
        temp.path(),
        &FileRoots::new(temp.path()),
        "no-such-artifact"
    )
    .await
    .is_err());
    // A path is not an id: the store's id validation refuses separators.
    assert!(
        resolve_source(temp.path(), &FileRoots::new(temp.path()), "/etc/passwd")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resolve_source_refuses_a_file_the_store_does_not_vouch_for() {
    use openhuman_rpc::embed::artifacts::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let temp = tempfile::tempdir().unwrap();
    let secret = temp.path().join("secret.txt");
    std::fs::write(&secret, b"private").unwrap();
    let meta = ArtifactMeta {
        id: "tampered".to_string(),
        kind: ArtifactKind::Other,
        title: "x".to_string(),
        path: "secret.txt".to_string(),
        file: Some(secret.to_string_lossy().into_owned()),
        file_root: Some(temp.path().join("Files").to_string_lossy().into_owned()),
        size_bytes: 7,
        status: ArtifactStatus::Ready,
        created_at: chrono::Utc::now(),
        error: None,
        thread_id: None,
        tool_call_id: None,
    };
    let dir = temp.path().join("artifacts").join("tampered");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(resolve_source(
        temp.path(),
        &FileRoots::new(temp.path().join("Files")),
        "tampered"
    )
    .await
    .is_err());
}

/// Download-by-id does not take a record's `file_root` on its own say-so: a
/// record claiming a home-directory root for a private file inside it is
/// refused because that root is not one of the vouched-for files folders.
#[tokio::test]
async fn resolve_source_refuses_a_record_that_claims_its_own_root() {
    use openhuman_rpc::embed::artifacts::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let private = home.join("private.pdf");
    std::fs::write(&private, b"private").unwrap();
    let meta = ArtifactMeta {
        id: "claims-home".to_string(),
        kind: ArtifactKind::Other,
        title: "x".to_string(),
        path: "private.pdf".to_string(),
        file: Some(private.to_string_lossy().into_owned()),
        file_root: Some(home.to_string_lossy().into_owned()),
        size_bytes: 7,
        status: ArtifactStatus::Ready,
        created_at: chrono::Utc::now(),
        error: None,
        thread_id: None,
        tool_call_id: None,
    };
    let dir = temp.path().join("artifacts").join("claims-home");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    let files = FileRoots::new(temp.path().join("Files"));
    assert!(resolve_source(temp.path(), &files, "claims-home")
        .await
        .is_err());
}

#[tokio::test]
async fn copy_to_path_copies_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src.pptx");
    let dst = temp.path().join("dst.pptx");
    std::fs::write(&src, b"deck-bytes").unwrap();
    let n = copy_to_path(&src, &dst).await.unwrap();
    assert_eq!(n, b"deck-bytes".len() as u64);
    assert_eq!(std::fs::read(&dst).unwrap(), b"deck-bytes");
}

#[tokio::test]
async fn download_rejects_bad_source() {
    // Source validation runs before the Downloads directory is touched,
    // so these resolve without writing anything.
    assert!(
        download_artifact_to_downloads(String::new(), "x.pptx".to_string())
            .await
            .is_err()
    );
    assert!(
        download_artifact_to_downloads("   ".to_string(), "x.pptx".to_string())
            .await
            .is_err()
    );
}

#[test]
fn split_stem_ext_pairs() {
    assert_eq!(
        split_stem_ext("file.pptx"),
        ("file".to_string(), "pptx".to_string())
    );
    assert_eq!(
        split_stem_ext("noext"),
        ("noext".to_string(), String::new())
    );
    assert_eq!(
        split_stem_ext(".hidden"),
        (".hidden".to_string(), String::new())
    );
    assert_eq!(
        split_stem_ext("trailing."),
        ("trailing.".to_string(), String::new())
    );
    assert_eq!(
        split_stem_ext("a.b.c"),
        ("a.b".to_string(), "c".to_string())
    );
}

#[test]
fn pick_unique_inserts_collision_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let first = pick_unique_path(dir, "deck.pptx");
    assert_eq!(first, dir.join("deck.pptx"));

    std::fs::write(&first, b"").unwrap();
    let second = pick_unique_path(dir, "deck.pptx");
    assert_eq!(second, dir.join("deck (1).pptx"));

    std::fs::write(&second, b"").unwrap();
    let third = pick_unique_path(dir, "deck.pptx");
    assert_eq!(third, dir.join("deck (2).pptx"));
}

#[test]
fn pick_unique_handles_no_extension() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let first = pick_unique_path(dir, "noext");
    assert_eq!(first, dir.join("noext"));
    std::fs::write(&first, b"").unwrap();
    let second = pick_unique_path(dir, "noext");
    assert_eq!(second, dir.join("noext (1)"));
}

#[tokio::test]
async fn download_rejects_invalid_inputs() {
    assert!(
        download_artifact_to_downloads(String::new(), "x.pptx".to_string())
            .await
            .is_err()
    );
    assert!(
        download_artifact_to_downloads("/tmp/x".to_string(), String::new())
            .await
            .is_err()
    );
    assert!(
        download_artifact_to_downloads("relative".to_string(), "x.pptx".to_string())
            .await
            .is_err()
    );
    assert!(
        download_artifact_to_downloads("/nope".to_string(), "../escape.pptx".to_string())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn save_as_refuses_before_the_dialog_opens() {
    let temp = tempfile::tempdir().unwrap();
    let files_dir = temp.path().join("Files");
    let id = ready_artifact(temp.path(), &files_dir).await;
    let roots = FileRoots::new(&files_dir);
    let never = |_: String| async { panic!("the dialog must not open") };

    for (bad_id, filename) in [
        ("", "Deck.pptx"),
        ("   ", "Deck.pptx"),
        ("no-such-id", "Deck.pptx"),
    ] {
        assert!(
            save_to_chosen_path(temp.path(), &roots, bad_id, filename, never)
                .await
                .is_err(),
            "id {bad_id:?} must be refused"
        );
    }
    for bad_name in ["../escape.pptx", "a/b.pptx", ""] {
        assert!(
            save_to_chosen_path(temp.path(), &roots, &id, bad_name, never)
                .await
                .is_err(),
            "filename {bad_name:?} must be refused"
        );
    }
}

#[tokio::test]
async fn save_as_copies_to_the_chosen_path_and_reports_a_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let files_dir = temp.path().join("Files");
    let id = ready_artifact(temp.path(), &files_dir).await;
    let roots = FileRoots::new(&files_dir);

    let cancelled = save_to_chosen_path(temp.path(), &roots, &id, "Deck.pptx", |_| async { None })
        .await
        .unwrap();
    assert_eq!(cancelled, None);

    let dest = temp.path().join("chosen").join("Deck.pptx");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let chosen = dest.clone();
    let saved = save_to_chosen_path(temp.path(), &roots, &id, "Deck.pptx", |name| async move {
        assert_eq!(name, "Deck.pptx");
        Some(chosen)
    })
    .await
    .unwrap();
    assert_eq!(saved.as_deref(), Some(dest.display().to_string().as_str()));
    assert_eq!(std::fs::read(&dest).unwrap(), b"deck");
}
