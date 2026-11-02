use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tiny_artifacts::{
    ArtifactAuthor, ArtifactKind, ArtifactRecord, ArtifactStore, ArtifactVersion, Author, Content,
    FileRef, FileStore, PublishOptions, PublishedLink,
};

use super::*;

struct UnusedPublisher;

#[derive(Default)]
struct MemoryArtifactStore(Mutex<HashMap<String, ArtifactRecord>>);

#[async_trait::async_trait]
impl ArtifactStore for MemoryArtifactStore {
    async fn list(&self) -> anyhow::Result<Vec<ArtifactRecord>> {
        Ok(self.0.lock().unwrap().values().cloned().collect())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<ArtifactRecord>> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    async fn upsert(&self, artifact: &ArtifactRecord) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(artifact.id.clone(), artifact.clone());
        Ok(())
    }

    async fn delete(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.0.lock().unwrap().remove(id).is_some())
    }
}

#[async_trait::async_trait]
impl tiny_artifacts::ArtifactPublisher for UnusedPublisher {
    async fn publish(
        &self,
        _record: &ArtifactRecord,
        _version: &ArtifactVersion,
        _options: &PublishOptions,
    ) -> anyhow::Result<PublishedLink> {
        anyhow::bail!("publisher is not used by this test")
    }
}

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
) -> crate::artifacts::version_documents::DocumentArtifactStore {
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
async fn legacy_path_refs_are_guarded_and_imported_as_opaque_file_keys() {
    let dir = tempfile::tempdir().unwrap();
    let source_root = dir.path().join("Files");
    let blobs_root = dir.path().join("blobs");
    std::fs::create_dir_all(&source_root).unwrap();
    let source = source_root.join("report.txt");
    std::fs::write(&source, "legacy bytes").unwrap();

    let records = Arc::new(MemoryArtifactStore::default());
    let base: Arc<dyn ArtifactStore> = records.clone();
    let blobs: Arc<dyn FileStore> = Arc::new(tiny_artifacts::FsFileStore::new(&blobs_root));
    let importing = LegacyFileImportStore {
        records: base,
        files: blobs.clone(),
        roots: guarded_roots(vec![source_root]),
    };
    let mut version = ArtifactVersion::new(
        1,
        source.display().to_string(),
        ArtifactAuthor::Agent,
        "agent",
        1,
    );
    version.file = Some(FileRef {
        key: source.display().to_string(),
        size_bytes: 12,
        media_type: None,
    });
    let mut record = ArtifactRecord::pending("a-legacy", "Report", ArtifactKind::File, 1);
    record.versions.push(version);
    records.upsert(&record).await.unwrap();

    let imported = importing.get("a-legacy").await.unwrap().unwrap();
    let file = imported.latest().unwrap().file.as_ref().unwrap();
    assert_eq!(file.key, "artifacts/files/a-legacy/v1.txt");
    assert_ne!(file.key, source.display().to_string());
    assert_eq!(blobs.read(&file.key).await.unwrap(), b"legacy bytes");

    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, "must stay outside").unwrap();
    let mut blocked_version = ArtifactVersion::new(
        1,
        outside.display().to_string(),
        ArtifactAuthor::Agent,
        "agent",
        1,
    );
    blocked_version.file = Some(FileRef {
        key: outside.display().to_string(),
        size_bytes: 17,
        media_type: None,
    });
    let mut blocked = ArtifactRecord::pending("a-outside", "Outside", ArtifactKind::File, 1);
    blocked.versions.push(blocked_version);
    records.upsert(&blocked).await.unwrap();
    assert!(importing.get("a-outside").await.is_err());
}

#[tokio::test]
async fn explicit_artifact_context_binds_records_and_blobs_to_the_same_profile() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let (workspace, files) = temp_dirs();
    let source = files.join("deck.pptx");
    std::fs::write(&source, b"profile-a-bytes").unwrap();
    let context_for = |profile: &'static str| {
        let overlay = ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .profile(profile);
        CoreContext::for_test(DomainSet::full(), None).derive_with(overlay)
    };
    let core_a = context_for("artifact-a");
    let core_b = context_for("ambient-b");
    let context_a = DispatchCtx::new(core_a.clone());
    let context_b = DispatchCtx::new(core_b.clone());

    CoreContext::scope(context_for("ambient-b"), async {
        let scoped_a = crate::storage::sessions::artifact_scoped_for_in(&workspace, &context_a)
            .expect("profile A scope");
        let (artifacts_a, files_a) = build_artifacts_service_with_scope(
            &workspace,
            FileRoots::new(&files),
            None,
            None,
            &context_a,
            Some(scoped_a),
            None,
        );
        let created = artifacts_a
            .create(
                ArtifactKind::File,
                "Deck",
                Content::File(source.display().to_string()),
                &Author::agent("agent-a"),
                None,
            )
            .await
            .unwrap();
        let id = created.record.id.clone();
        let file = created.record.latest().unwrap().file.as_ref().unwrap();
        assert_eq!(
            files_a.as_ref().unwrap().read(&file.key).await.unwrap(),
            b"profile-a-bytes"
        );

        let scoped_b = crate::storage::sessions::artifact_scoped_for_in(&workspace, &context_b)
            .expect("profile B scope");
        let (artifacts_b, files_b) = build_artifacts_service_with_scope(
            &workspace,
            FileRoots::new(&files),
            None,
            None,
            &context_b,
            Some(scoped_b),
            None,
        );
        assert!(artifacts_b.get(&id).await.unwrap().is_none());
        assert!(files_b
            .as_ref()
            .unwrap()
            .metadata(&file.key)
            .await
            .unwrap()
            .is_none());

        let card_a = CoreContext::scope(core_a.clone(), async {
            super::super::store::get_artifact(&workspace, &id).await
        })
        .await;
        assert!(
            card_a.is_ok(),
            "the explicit artifact profile owns its UI card"
        );
        let card_b = CoreContext::scope(core_b.clone(), async {
            super::super::store::get_artifact(&workspace, &id).await
        })
        .await;
        assert!(
            card_b.is_err(),
            "the ambient profile must not receive the UI card"
        );
    })
    .await;
}

#[tokio::test]
async fn missing_artifact_storage_binding_does_not_fall_back_to_ambient_records() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let (workspace, files) = temp_dirs();
    let context_for = |profile: &'static str| {
        let overlay = ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .profile(profile);
        CoreContext::for_test(DomainSet::full(), None).derive_with(overlay)
    };
    let core_b = context_for("ambient-b");
    let context_b = DispatchCtx::new(core_b.clone());

    CoreContext::scope(core_b, async {
        let ambient_artifacts =
            artifacts_service(&workspace, FileRoots::new(&files), None, None, &context_b);
        ambient_artifacts
            .create(
                ArtifactKind::Text,
                "Ambient private",
                Content::Text("only profile B".into()),
                &Author::agent("agent-b"),
                None,
            )
            .await
            .unwrap();

        // A failed tenant binding is represented directly at the production
        // builder seam, avoiding process-global SaaS-mode configuration.
        let (unbound_artifacts, _) = build_artifacts_service_with_scope(
            &workspace,
            FileRoots::new(&files),
            None,
            None,
            &context_b,
            None,
            None,
        );
        assert!(
            unbound_artifacts.list().await.is_err(),
            "a missing scoped binding must not expose the ambient profile's records"
        );
    })
    .await;
}

#[tokio::test]
async fn legacy_bridge_does_not_overwrite_an_existing_card_file_outside_trusted_roots() {
    let (workspace, files) = temp_dirs();
    let root = files.parent().unwrap();
    let records: Arc<dyn ArtifactStore> = Arc::new(MemoryArtifactStore::default());
    let file_store: Arc<dyn FileStore> =
        Arc::new(tiny_artifacts::FsFileStore::new(root.join("blobs")));
    let ctx = crate::core::runtime::DispatchCtx::unbound();
    let bridge = Arc::new(LegacyUiBridge::new(
        &workspace,
        FileRoots::new(&files),
        guarded_roots(vec![files.clone()]),
        Some(file_store.clone()),
        None,
        &ctx,
    ));
    let artifacts = Artifacts::new(records)
        .with_file_roots(guarded_roots(vec![files.clone()]))
        .with_file_store(file_store)
        .with_observer(bridge);

    let source = files.join("source.txt");
    std::fs::write(&source, b"first version").unwrap();
    let saved = artifacts
        .create(
            ArtifactKind::File,
            "Protected export",
            Content::File(source.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();

    let outside = root.join("poisoned.txt");
    std::fs::write(&outside, b"leave this file alone").unwrap();
    let mut card = super::super::store::get_artifact(&workspace, &saved.record.id)
        .await
        .unwrap();
    card.file = Some(outside.display().to_string());
    super::super::store::save_artifact_meta(&workspace, &card)
        .await
        .unwrap();

    std::fs::write(&source, b"replacement version").unwrap();
    let revised = artifacts
        .revise(
            &saved.record.id,
            Content::File(source.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();

    assert!(
        revised.notice.is_some(),
        "a poisoned legacy file path should be refused by the bridge"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"leave this file alone",
        "the bridge must not write outside its trusted files roots"
    );
}

#[tokio::test]
async fn an_edit_is_appended_as_an_operator_version_and_written_to_the_file() {
    let (workspace, files) = temp_dirs();
    let store = seeded(&workspace, &files).await;

    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let saved = save_operator_version(&workspace, FileRoots::new(&files), "a-1", "# v2", &ctx)
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

    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let saved = save_operator_version(&workspace, FileRoots::new(&files), "a-1", "# v1", &ctx)
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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let err = save_operator_version(&workspace, FileRoots::new(&files), "nope", "x", &ctx)
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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(
        &workspace,
        FileRoots::new(&files),
        Some(&action),
        None,
        &ctx,
    );

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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), None, None, &ctx);
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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(&workspace, FileRoots::new(&files), None, None, &ctx);
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

#[test]
fn share_artifact_is_offered_only_with_a_publisher() {
    let (workspace, files) = temp_dirs();
    let names = |publisher_available: bool| {
        registered_tools(
            &workspace,
            FileRoots::new(&files),
            &workspace,
            publisher_available.then(|| {
                std::sync::Arc::new(
                    |_| -> std::sync::Arc<dyn tiny_artifacts::ArtifactPublisher> {
                        std::sync::Arc::new(UnusedPublisher)
                    },
                ) as crate::artifacts::ArtifactPublisherBinder
            }),
        )
        .iter()
        .map(|t| t.name().to_string())
        .collect::<Vec<_>>()
    };
    let without = names(false);
    assert!(
        !without.contains(&"share_artifact".to_string()),
        "{without:?}"
    );
    assert!(without.contains(&"delete_artifact".to_string()));
    let with = names(true);
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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(&workspace, roots, None, None, &ctx);
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
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(
        &workspace,
        FileRoots::new(&files),
        Some(&action),
        None,
        &ctx,
    );

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
fn a_stored_file_key_resolves_under_the_version_files_folder_and_an_absolute_one_stays() {
    let workspace = Path::new("/ws");
    let kept = tiny_artifacts::FileRef {
        key: "artifacts/files/a1/v2.pptx".into(),
        size_bytes: 1,
        media_type: None,
    };
    assert_eq!(
        stored_file_path(workspace, &kept),
        version_files_dir(workspace).join("artifacts/files/a1/v2.pptx")
    );
    let legacy = tiny_artifacts::FileRef {
        key: "/elsewhere/old.pptx".into(),
        ..kept
    };
    assert_eq!(
        stored_file_path(workspace, &legacy),
        PathBuf::from("/elsewhere/old.pptx")
    );
}
