use super::*;
use crate::memory::sources::state;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use tinymemory_api::{ItemKind, MetaFilter};
use tinymemory_integrations::sources::{SourceState, SourceStatus};

fn folder_source(id: &str, dir: &std::path::Path, mins: Option<u32>) -> MemorySourceConfig {
    MemorySourceConfig {
        id: id.to_string(),
        kind: MemorySourceKind::Folder,
        target: dir.display().to_string(),
        label: id.to_string(),
        schedule_mins: mins,
        namespace: None,
    }
}

/// A workspace with `n` markdown notes under `<action_dir>/notes`.
fn notes(config: &Config, n: usize) -> PathBuf {
    let dir = config.action_dir.join("notes");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..n {
        std::fs::write(
            dir.join(format!("note-{i}.md")),
            format!("# Note {i}\n\nunique content number {i} about gardening"),
        )
        .unwrap();
    }
    dir
}

async fn wait_for_idle(config: &Config, id: &str) -> SourceState {
    for _ in 0..400 {
        if let Some(state) = state::load(&config.workspace_dir).get(id) {
            if state.status != SourceStatus::Syncing {
                return state.clone();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("sync of {id} never finished");
}

#[test]
fn validate_accepts_every_kind_and_refuses_an_empty_target() {
    let mk = |kind, target: &str| MemorySourceConfig {
        id: "s".into(),
        kind,
        target: target.into(),
        label: "L".into(),
        schedule_mins: None,
        namespace: None,
    };
    for (kind, target) in [
        (MemorySourceKind::Folder, "/p"),
        (MemorySourceKind::File, "/p/a.md"),
        (MemorySourceKind::Link, "https://example.com/"),
        (MemorySourceKind::Github, "https://github.com/o/r"),
        (MemorySourceKind::Rss, "https://example.com/feed"),
    ] {
        validate(&mk(kind, target)).unwrap();
    }
    assert_eq!(
        validate(&mk(MemorySourceKind::Folder, ""))
            .unwrap_err()
            .code(),
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn sync_one_stores_each_file_as_a_document_tagged_with_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let dir = notes(&config, 3);
    let stored_count = sync_one(&config, &folder_source("src-notes", &dir, None))
        .await
        .unwrap();
    assert_eq!(stored_count, 3);
    let docs = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Document],
            source_id: Some("src-notes".into()),
            ..MetaFilter::default()
        },
    )
    .await;
    assert_eq!(docs.len(), 3);
}

#[tokio::test]
async fn sync_one_reports_memory_off_and_unreadable_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let dir = notes(&config, 1);
    let off = sync_one(&config, &folder_source("s", &dir, None))
        .await
        .unwrap_err();
    assert_eq!(off.code(), "MEMORY_OFF");

    bind_reference(&config);
    let missing = config.action_dir.join("does-not-exist");
    assert!(sync_one(&config, &folder_source("s", &missing, None))
        .await
        .is_err());
}

#[tokio::test]
async fn start_sync_walks_syncing_then_idle_and_records_items() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    let dir = notes(&config, 2);
    config
        .memory
        .sources
        .push(folder_source("src-ok", &dir, None));

    let started = start_sync(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some("src-ok"),
    )
    .await
    .unwrap();
    assert_eq!(started, vec!["src-ok".to_string()]);
    let done = wait_for_idle(&config, "src-ok").await;
    assert_eq!(done.status, SourceStatus::Idle);
    assert_eq!(done.items, 2);
    assert!(done.last_sync_at.is_some());
    assert!(done.error.is_none());

    // Listed through the registry view too.
    let view = crate::memory::sources::list(&config);
    assert_eq!(view[0].items, 2);
    assert_eq!(view[0].status, SourceStatus::Idle);
}

#[tokio::test]
async fn a_failing_sync_lands_in_error_with_the_message() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    let missing = config.action_dir.join("vanished");
    config
        .memory
        .sources
        .push(folder_source("src-bad", &missing, None));

    start_sync(&crate::core::runtime::DispatchCtx::ambient(), &config, None)
        .await
        .unwrap();
    let done = wait_for_idle(&config, "src-bad").await;
    assert_eq!(done.status, SourceStatus::Error);
    assert!(done.error.as_deref().is_some_and(|e| !e.is_empty()));
    assert!(done.last_sync_at.is_some());
}

#[tokio::test]
async fn start_sync_refuses_unknown_ids_and_memory_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(
        start_sync(&crate::core::runtime::DispatchCtx::ambient(), &config, None)
            .await
            .unwrap_err()
            .code(),
        "MEMORY_OFF"
    );
    bind_reference(&config);
    assert_eq!(
        start_sync(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Some("src-nope")
        )
        .await
        .unwrap_err()
        .code(),
        "INVALID_REQUEST"
    );
    assert!(
        start_sync(&crate::core::runtime::DispatchCtx::ambient(), &config, None)
            .await
            .unwrap()
            .is_empty(),
        "no sources, nothing started"
    );
}

#[tokio::test]
async fn a_source_already_syncing_is_not_started_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    let dir = notes(&config, 1);
    config
        .memory
        .sources
        .push(folder_source("src-once", &dir, None));
    // Hold a claim as a running sync would: a second request starts nothing.
    let held = syncer_for(&config).start(Some("src-once")).unwrap();
    assert_eq!(held.len(), 1);
    assert!(start_sync(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some("src-once")
    )
    .await
    .unwrap()
    .is_empty());
    drop(held);
    assert_eq!(
        start_sync(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Some("src-once")
        )
        .await
        .unwrap()
        .len(),
        1
    );
    wait_for_idle(&config, "src-once").await;
}

#[tokio::test]
async fn sync_due_starts_only_scheduled_sources_that_are_due() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    let dir = notes(&config, 1);
    config
        .memory
        .sources
        .push(folder_source("src-sched", &dir, Some(15)));
    config
        .memory
        .sources
        .push(folder_source("src-manual", &dir, None));
    config
        .memory
        .sources
        .push(folder_source("src-fresh", &dir, Some(15)));
    let now = Utc::now();
    state::update(&config.workspace_dir, "src-fresh", |s| {
        s.last_sync_at = Some(now);
    });

    let started = sync_due(&crate::core::runtime::DispatchCtx::ambient(), &config, now).await;
    assert_eq!(started, vec!["src-sched".to_string()]);
    wait_for_idle(&config, "src-sched").await;

    // Fifteen minutes later everything scheduled is due again; manual never is.
    let later = sync_due(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        now + chrono::Duration::minutes(16),
    )
    .await;
    assert!(later.contains(&"src-sched".to_string()));
    assert!(later.contains(&"src-fresh".to_string()));
    assert!(!later.contains(&"src-manual".to_string()));
    wait_for_idle(&config, "src-sched").await;
    wait_for_idle(&config, "src-fresh").await;
}

#[tokio::test]
async fn sync_due_with_memory_off_starts_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config
        .memory
        .sources
        .push(folder_source("s", tmp.path(), Some(15)));
    assert!(sync_due(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Utc::now()
    )
    .await
    .is_empty());
}

#[test]
fn reset_interrupted_marks_syncing_sources_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    state::update(ws, "a", |s| s.status = SourceStatus::Syncing);
    state::update(ws, "b", |s| {
        s.status = SourceStatus::Error;
        s.error = Some("kept".into());
    });
    state::reset_interrupted(ws);
    let all = state::load(ws);
    assert_eq!(all["a"].status, SourceStatus::Idle);
    assert_eq!(all["b"].status, SourceStatus::Error);
    assert_eq!(all["b"].error.as_deref(), Some("kept"));
    state::remove(ws, "a");
    state::remove(ws, "never-there");
    assert!(!state::load(ws).contains_key("a"));
}

#[tokio::test]
async fn a_sync_queues_a_belief_build_only_for_an_engine_that_waits_for_one() {
    use std::sync::Arc;
    use tinymemory_api::conformance::ReferenceEngine;
    use tinymemory_api::Consolidation;

    let note = || {
        tinymemory_api::StoreItem::document(
            "Quarterly planning notes",
            tinymemory_api::MemoryMeta::default(),
        )
    };
    for (consolidation, queued) in [(Consolidation::OnDemand, 1), (Consolidation::Automatic, 0)] {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_in(&tmp);
        let bound = engine::BoundEngine {
            engine: Arc::new(ReferenceEngine::new().with_consolidation(consolidation)),
            id: "reference".into(),
            endpoint: "memory://reference".into(),
        };
        let stored = place_and_store(
            &config,
            &bound,
            vec![note()],
            (MemorySourceKind::Folder, "src"),
            &tinymemory_tools::MemoryLayout::default(),
        )
        .await
        .unwrap();
        assert_eq!(stored, 1);
        let pending = crate::memory::lifecycle::jobs::snapshot(&config)
            .await
            .pending
            .len();
        assert_eq!(pending, queued, "{consolidation:?}");
    }
}

#[tokio::test]
async fn github_files_per_repository_only_when_switched_on() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let engine = bind_reference(&config);
    let bound = engine::resolve(&config).await.engine().unwrap();
    let issue = |repo: &str, text: &str| {
        let mut meta = tinymemory_api::MemoryMeta::default();
        meta.repo = Some(repo.to_string());
        tinymemory_api::StoreItem::document(text, meta)
    };
    let sync = |config: Config, items| {
        let bound = bound.clone();
        async move {
            place_and_store(
                &config,
                &bound,
                items,
                (MemorySourceKind::Github, "src"),
                &tinymemory_tools::MemoryLayout::default(),
            )
            .await
            .unwrap()
        }
    };
    let nodes = || async {
        let mut nodes: Vec<String> = stored(&engine, MetaFilter::kinds([ItemKind::Document]))
            .await
            .into_iter()
            .map(|hit| hit.meta.namespace.to_string())
            .collect();
        nodes.sort();
        nodes
    };

    // Off: one GitHub source.
    config.memory.split_github_by_repo = false;
    sync(config.clone(), vec![issue("acme/api", "off")]).await;
    assert_eq!(nodes().await, ["source:github"]);

    config.memory.split_github_by_repo = true;
    sync(
        config.clone(),
        vec![issue("Acme/API", "one"), issue("acme/web", "two")],
    )
    .await;
    assert_eq!(
        nodes().await,
        [
            "source:github",
            "source:github/project:acme--api",
            "source:github/project:acme--web"
        ]
    );
    assert_eq!(
        crate::memory::lifecycle::jobs::snapshot(&config)
            .await
            .pending
            .len(),
        3,
        "one belief build per node written"
    );
}
