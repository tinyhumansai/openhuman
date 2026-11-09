use super::*;

fn gate_for(workspace: &Path) -> DiscoveryGate {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    DiscoveryGate::from_config(
        &DispatchCtx::host_default(),
        &config,
        Some(DomainSet::full()),
    )
}

fn resources(reference: &str) -> Vec<ResourceFile> {
    vec![
        ResourceFile {
            path: tinyskills::WORKFLOW_MD.to_string(),
            contents: "---\nname: Flow authoring\ndescription: module manual\n---\n\nRead references when needed.\n"
                .to_string(),
        },
        ResourceFile {
            path: "references/guide.md".to_string(),
            contents: reference.to_string(),
        },
    ]
}

#[tokio::test]
async fn optional_warmup_returns_while_the_loader_is_blocked_and_deduplicates() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    crate::modules::tinyflows::copilot_proxy(&config)
        .await
        .expect("linked TinyFlows fixture is admitted and serving");
    let context = crate::core::runtime::CoreContext::for_test(DomainSet::full(), None).derive_with(
        crate::core::runtime::ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .profile("blocked-prewarm")
        .without_user_skill_roots(),
    );
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();

    crate::core::runtime::CoreContext::scope(context, async {
        let ctx = DispatchCtx::ambient();
        assert!(schedule_prewarm_operation(
            &ctx,
            &config,
            Some(DomainSet::full()),
            async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
                let _ = finished_tx.send(());
            },
        ));
        // The scheduled operation is now blocked on `release_rx`. Reaching
        // this point proves the caller did not wait for the loader.
        started_rx.await.expect("background task started");
        assert!(
            !schedule_prewarm_operation(&ctx, &config, Some(DomainSet::full()), async {}),
            "a second warmup for the same explicit scope is coalesced"
        );
        release_tx.send(()).expect("release background task");
        finished_rx.await.expect("background task finished");
    })
    .await;
}

#[test]
fn resource_snapshot_revisions_follow_changed_bus_bytes() {
    let workspace = tempfile::tempdir().expect("workspace");
    let gate = gate_for(workspace.path());
    let first = publish_snapshot(workspace.path(), &gate, resources("first version"))
        .expect("materialize first resource set");
    let unchanged = publish_snapshot(workspace.path(), &gate, resources("first version"))
        .expect("retain identical resource set");
    assert_eq!(unchanged, first, "unchanged bus files keep their revision");

    let updated = publish_snapshot(workspace.path(), &gate, resources("second version"))
        .expect("materialize updated resource set");
    assert!(
        updated > first,
        "changed bus bytes advance the snapshot revision"
    );
    let current = snapshot_for(&snapshot_key(workspace.path(), &gate), updated).expect("snapshot");
    assert!(materialization_matches(workspace.path(), &gate, &current));
    assert_eq!(current.files[1].contents, "second version");
}

#[test]
fn tiny_skills_rejects_unsafe_resource_paths_without_writing_outside_scope() {
    let workspace = tempfile::tempdir().expect("workspace");
    let gate = gate_for(workspace.path());
    let mut files = resources("safe reference");
    files.push(ResourceFile {
        path: "../outside.txt".to_string(),
        contents: "must stay outside".to_string(),
    });

    assert!(publish_snapshot(workspace.path(), &gate, files).is_err());
    assert!(!workspace.path().join(".openhuman/outside.txt").exists());
    assert!(snapshot_for(&snapshot_key(workspace.path(), &gate), 1).is_none());
}

/// While the local legacy flow import is incomplete, workflow access is paused
/// (`ensure_flow_scope_ready`), so the builtin must stay hidden; once the
/// import finishes, the next prewarm must surface it.
#[cfg(feature = "storage-sqlite")]
#[tokio::test]
// The guard serializes the process-wide storage slot for this async test.
#[allow(clippy::await_holding_lock)]
async fn builtin_is_hidden_until_the_legacy_flow_import_completes() {
    use crate::core::runtime::{ComponentSet, ContextOverlay, CoreContext};
    use crate::storage::Scope;

    let workspace = tempfile::tempdir().expect("workspace");
    let legacy = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/storage/import/fixtures/tinyflows/workspace/flows");
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy_tree(&legacy, &workspace.path().join("flows"));
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    let backend = crate::storage::test_support::sqlite_backend().expect("sqlite backend");
    let _storage =
        crate::storage::test_support::install_backend_for_test(Arc::clone(&backend.backend));
    let context = CoreContext::for_test_with_components(ComponentSet::all()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .without_user_skill_roots(),
    );

    CoreContext::scope(context, async {
        let ctx = DispatchCtx::ambient();
        let visible = |ctx: &DispatchCtx| {
            crate::skills::load_workflow_metadata_for(ctx, &config)
                .iter()
                .any(|skill| skill.id() == FLOW_SKILL_ID)
        };

        prewarm(&ctx, &config, None).await;
        assert!(
            !visible(&ctx),
            "flow-authoring must stay hidden while the legacy flow import is incomplete"
        );

        let imported = crate::storage::import::tinyflows::import_flows(
            &backend.backend,
            &Scope::local(),
            &config,
        )
        .expect("legacy flow import");
        assert!(
            matches!(imported, crate::storage::import::Outcome::Imported { .. }),
            "{imported:?}"
        );

        prewarm(&ctx, &config, None).await;
        assert!(
            visible(&ctx),
            "flow-authoring must appear once the legacy flow import has completed"
        );
    })
    .await;
}
