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
        // Mirrors TinySecurity's admission: a media file inside the artifacts
        // reservation is an exception of it; one filed elsewhere is an exact
        // read-only trusted root (an exception there would be refused).
        let is_media_exception = policy
            .reserved_paths
            .iter()
            .any(|reservation| reservation.exceptions.contains(&path))
            || policy.trusted_roots.iter().any(|root| {
                root.path == path && root.access == tinysecurity_bus::PathAccess::ReadOnly
            });
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

/// TinySecurity refuses a whole path policy when a reservation exception is not
/// a strict descendant of its reservation. A ready media artifact filed in the
/// Files folder (outside `<workspace>/artifacts`) must therefore never be sent as
/// an artifacts-reservation exception; it is an exact read-only trusted root.
#[test]
fn media_artifact_outside_the_artifacts_reservation_is_a_read_only_trusted_root() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    let filed = tmp.path().join("Files").join("image.png");
    let inside = workspace.join("artifacts").join("image.png");
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: tmp.path().join("acting"),
        ..SecurityPolicy::default()
    };
    policy
        .media_artifact_paths
        .lock()
        .unwrap()
        .extend([filed.clone(), inside.clone()]);

    let native = policy.native_path_policy();

    for reservation in &native.reserved_paths {
        for exception in &reservation.exceptions {
            assert!(
                exception != &reservation.path && exception.starts_with(&reservation.path),
                "exception {exception:?} is outside reservation {:?}",
                reservation.path
            );
        }
    }
    let artifacts = native
        .reserved_paths
        .iter()
        .find(|reservation| reservation.path == workspace.join("artifacts"))
        .expect("artifacts reservation");
    assert!(artifacts.exceptions.contains(&inside));
    assert!(!artifacts.exceptions.contains(&filed));
    assert!(native.trusted_roots.iter().any(|root| {
        root.path == filed && root.access == tinysecurity_bus::PathAccess::ReadOnly
    }));
    assert!(!native.trusted_roots.iter().any(|root| root.path == inside));
}
