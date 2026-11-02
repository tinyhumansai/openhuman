use super::*;
use crate::artifacts::{create_artifact, finalize_artifact, ArtifactKind, FileRoots};
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Debug)]
struct MediaPathPort;

#[async_trait::async_trait]
impl crate::modules::security::gap::PathValidationPort for MediaPathPort {
    async fn validate_host_path(
        &self,
        policy: tinysecurity_bus::PathPolicy,
        path: String,
        _parent: bool,
    ) -> Result<tinysecurity_bus::PathValidationResult, String> {
        let path = std::path::PathBuf::from(path);
        let is_media_exception = policy
            .reserved_paths
            .iter()
            .any(|reservation| reservation.exceptions.contains(&path));
        if is_media_exception {
            let resolved = tokio::fs::canonicalize(&path).await.unwrap_or(path);
            Ok(tinysecurity_bus::PathValidationResult::Allowed { resolved })
        } else {
            Ok(tinysecurity_bus::PathValidationResult::Denied {
                category: tinysecurity_bus::PathErrorCategory::PolicyDenied,
            })
        }
    }
}

#[tokio::test]
async fn ready_media_artifacts_are_readable_but_other_files_are_not() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let _validation =
        crate::modules::security::gap::install_path_validation_port(Arc::new(MediaPathPort));
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

    crate::artifacts::store::delete_artifact(&workspace, FileRoots::new(files), &image.id)
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
