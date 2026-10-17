use std::path::PathBuf;

use tiny_artifacts::{
    ArtifactAuthor, ArtifactKind, ArtifactRecord, ArtifactStore, Author, Content,
};

use super::*;

fn temp_dirs() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("tiny-store-{}", uuid::Uuid::new_v4()));
    let (workspace, files) = (root.join("workspace"), root.join("Files"));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    (workspace, files)
}

async fn seeded(
    workspace: &std::path::Path,
    _files: &std::path::Path,
) -> crate::agent::artifacts::version_documents::DocumentArtifactStore {
    let store = versioned_store(workspace);
    let record = ArtifactRecord::new(
        "a-1",
        "Notes",
        ArtifactKind::Markdown,
        "# v1",
        "agent",
        1_000,
    );
    store.upsert(&record).await.unwrap();
    store
}

#[tokio::test]
async fn an_edit_is_appended_as_an_operator_version_and_written_to_the_file() {
    let (workspace, files) = temp_dirs();
    let store = seeded(&workspace, &files).await;

    let saved = save_operator_version(&workspace, FileRoots::new(&files), "a-1", "# v2")
        .await
        .unwrap();
    assert_eq!(
        saved,
        SavedVersion {
            version: 2,
            saved: true
        }
    );

    let record = store.get("a-1").await.unwrap().unwrap();
    let latest = record.latest().unwrap();
    assert_eq!(latest.body, "# v2");
    assert_eq!(latest.author, ArtifactAuthor::Operator);
    assert_eq!(latest.author_id, OPERATOR_AUTHOR_ID);
    assert!(record.human_edit_diff().is_some(), "the edit is diff-able");

    // The visible file now holds the edit.
    let written = std::fs::read_to_string(files.join("notes.md")).unwrap();
    assert_eq!(written, "# v2");
}

#[tokio::test]
async fn an_unchanged_body_appends_nothing() {
    let (workspace, files) = temp_dirs();
    let store = seeded(&workspace, &files).await;

    let saved = save_operator_version(&workspace, FileRoots::new(&files), "a-1", "# v1")
        .await
        .unwrap();
    assert_eq!(
        saved,
        SavedVersion {
            version: 1,
            saved: false
        }
    );
    assert_eq!(store.get("a-1").await.unwrap().unwrap().versions.len(), 1);
}

#[tokio::test]
async fn a_missing_artifact_is_an_error() {
    let (workspace, files) = temp_dirs();
    let err = save_operator_version(&workspace, FileRoots::new(&files), "nope", "x")
        .await
        .unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[tokio::test]
async fn an_image_filed_through_the_service_gets_a_card_and_a_copy_in_files() {
    let (workspace, files) = temp_dirs();
    let action = workspace.parent().unwrap().join("projects");
    std::fs::create_dir_all(&action).unwrap();
    std::fs::write(action.join("chart.png"), b"png-bytes").unwrap();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), Some(&action), None);

    let saved = artifacts
        .create(
            ArtifactKind::Image,
            "Chart",
            Content::File("chart.png".into()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();
    assert!(saved.notice.is_none(), "{:?}", saved.notice);

    // The UI's row: ready, an image, with the file copied into Files.
    let meta = super::super::store::get_artifact(&workspace, &saved.record.id)
        .await
        .unwrap();
    assert_eq!(meta.status, super::super::types::ArtifactStatus::Ready);
    assert_eq!(meta.kind, super::super::types::ArtifactKind::Image);
    assert_eq!(
        std::fs::read(files.join("chart.png")).unwrap(),
        b"png-bytes"
    );
    assert_eq!(meta.size_bytes, 9);
}

#[tokio::test]
async fn a_file_outside_the_allowed_folders_is_refused() {
    let (workspace, files) = temp_dirs();
    let elsewhere = workspace.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("x.pdf"), b"x").unwrap();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), None, None);
    let err = artifacts
        .create(
            ArtifactKind::File,
            "X",
            Content::File(elsewhere.join("x.pdf").display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("outside"), "{err}");
}

#[tokio::test]
async fn deleting_an_artifact_removes_its_card() {
    let (workspace, files) = temp_dirs();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), None, None);
    let saved = artifacts
        .create(
            ArtifactKind::Markdown,
            "Notes",
            Content::Text("# hi".into()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();
    let id = saved.record.id;
    assert!(super::super::store::get_artifact(&workspace, &id)
        .await
        .is_ok());

    let (removed, notice) = artifacts.delete(&id).await.unwrap();
    assert!(removed);
    assert!(notice.is_none(), "{notice:?}");
    assert!(super::super::store::get_artifact(&workspace, &id)
        .await
        .is_err());
}

struct NoopPublisher;

#[async_trait::async_trait]
impl tiny_artifacts::ArtifactPublisher for NoopPublisher {
    async fn publish(
        &self,
        _record: &ArtifactRecord,
        _version: &tiny_artifacts::ArtifactVersion,
        options: &tiny_artifacts::PublishOptions,
    ) -> anyhow::Result<tiny_artifacts::PublishedLink> {
        Ok(tiny_artifacts::PublishedLink {
            url: "https://example/x".into(),
            expires_at_millis: None,
            published_at_millis: 0,
            public: options.public,
        })
    }
}

#[test]
fn share_artifact_is_offered_only_with_a_publisher() {
    let (workspace, files) = temp_dirs();
    let names = |publisher: Option<std::sync::Arc<dyn tiny_artifacts::ArtifactPublisher>>| {
        registered_tools(&workspace, FileRoots::new(&files), &workspace, publisher)
            .iter()
            .map(|t| t.name().to_string())
            .collect::<Vec<_>>()
    };
    let without = names(None);
    assert!(
        !without.contains(&"share_artifact".to_string()),
        "{without:?}"
    );
    assert!(without.contains(&"delete_artifact".to_string()));
    let with = names(Some(std::sync::Arc::new(NoopPublisher)));
    assert!(with.contains(&"share_artifact".to_string()), "{with:?}");
}

#[tokio::test]
async fn a_file_in_an_earlier_files_folder_can_still_be_revised() {
    let (workspace, files) = temp_dirs();
    let earlier = files.parent().unwrap().join("OldFiles");
    std::fs::create_dir_all(&earlier).unwrap();
    let old = earlier.join("deck.pptx");
    std::fs::write(&old, b"v1").unwrap();
    // The files folder moved; the earlier one stays trusted.
    let roots = FileRoots::new(&files).with_trusted([earlier.clone()]);
    let artifacts = artifacts_service(&workspace, roots, None, None);
    let id = artifacts
        .create(
            ArtifactKind::File,
            "Deck",
            Content::File(old.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap()
        .record
        .id;
    std::fs::write(&old, b"v2").unwrap();
    let saved = artifacts
        .revise(
            &id,
            Content::File(old.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(saved.version, 2);
}

#[tokio::test]
async fn a_protected_file_inside_the_working_folder_is_refused() {
    let (workspace, files) = temp_dirs();
    let action = workspace.parent().unwrap().join("home");
    let ssh = action.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ed25519"), b"PRIVATE").unwrap();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), Some(&action), None);

    let err = artifacts
        .create(
            ArtifactKind::File,
            "Key",
            Content::File(".ssh/id_ed25519".into()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("protected location"), "{err}");
}

#[test]
fn a_working_folder_that_contains_the_workspace_is_not_trusted() {
    let (workspace, files) = temp_dirs();
    let home = workspace.parent().unwrap().to_path_buf();
    let roots = file_roots(&workspace, &FileRoots::new(&files), Some(&home));
    assert!(!roots.contains(&home), "{roots:?}");
    assert!(roots.contains(&files));
    // A working folder beside the workspace stays the first root.
    let beside = home.join("projects");
    let roots = file_roots(&workspace, &FileRoots::new(&files), Some(&beside));
    assert_eq!(roots.first(), Some(&beside));
}

#[test]
fn a_protected_location_is_refused_under_its_macos_canonical_name() {
    // `/etc` canonicalizes to `/private/etc` on macOS.
    assert!(refused_path(Path::new("/private/etc/hosts")).is_some());
    assert!(refused_path(Path::new("/etc/hosts")).is_some());
    // `/private/var` and `/private/tmp` (temp folders) are not protected.
    assert!(refused_path(Path::new("/private/var/folders/x/notes.md")).is_none());
    assert!(refused_path(Path::new("/private/tmp/notes.md")).is_none());
    assert!(refused_path(Path::new("/Users/me/OpenHuman/notes.md")).is_none());
}
