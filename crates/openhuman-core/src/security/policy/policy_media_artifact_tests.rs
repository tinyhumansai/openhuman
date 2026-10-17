use super::*;
use crate::agent::artifacts::{create_artifact, finalize_artifact, ArtifactKind, FileRoots};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn ready_media_artifacts_are_readable_but_other_files_are_not() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    let files = tmp.path().join("Files");
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&files).await.unwrap();

    let (image, image_path) = create_artifact(
        &workspace,
        &files,
        ArtifactKind::Image,
        "Generated image",
        "png",
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(&image_path, b"image bytes").await.unwrap();
    finalize_artifact(&workspace, &image.id, 11, None)
        .await
        .unwrap();

    let (document, document_path) = create_artifact(
        &workspace,
        &files,
        ArtifactKind::Document,
        "Other file",
        "txt",
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(&document_path, b"document bytes")
        .await
        .unwrap();
    finalize_artifact(&workspace, &document.id, 14, None)
        .await
        .unwrap();

    let (_pending_video, pending_path) = create_artifact(
        &workspace,
        &files,
        ArtifactKind::Video,
        "Pending video",
        "mp4",
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(&pending_path, b"pending bytes")
        .await
        .unwrap();

    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: tmp.path().join("action"),
        media_files_roots: Arc::new(std::sync::Mutex::new(Some(FileRoots::new(files.clone())))),
        ..SecurityPolicy::default()
    };

    assert!(policy
        .validate_path(&image_path.to_string_lossy())
        .await
        .is_ok());
    assert!(policy
        .validate_path(&document_path.to_string_lossy())
        .await
        .is_err());
    assert!(policy
        .validate_path(&pending_path.to_string_lossy())
        .await
        .is_err());

    crate::agent::artifacts::store::delete_artifact(&workspace, FileRoots::new(files), &image.id)
        .await
        .unwrap();
    tokio::fs::write(&image_path, b"unrelated replacement")
        .await
        .unwrap();
    assert!(
        policy
            .validate_path(&image_path.to_string_lossy())
            .await
            .is_err(),
        "a cached path must not authorize a replacement file"
    );
}
