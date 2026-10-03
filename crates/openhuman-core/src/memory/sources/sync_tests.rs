use super::*;
use crate::memory::error::INVALID_REQUEST;
use crate::memory::sources::state;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use tinymemory::{ItemKind, MetaFilter};

fn folder_source(id: &str, dir: &std::path::Path, mins: Option<u32>) -> MemorySourceConfig {
    MemorySourceConfig {
        id: id.to_string(),
        kind: MemorySourceKind::Folder,
        target: dir.display().to_string(),
        label: id.to_string(),
        schedule_mins: mins,
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

async fn wait_for_idle(config: &Config, id: &str) -> state::SourceState {
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
fn reader_entry_maps_every_kind() {
    let mk = |kind, target: &str| MemorySourceConfig {
        id: "s".into(),
        kind,
        target: target.into(),
        label: "L".into(),
        schedule_mins: None,
    };
    let folder = reader_entry(&mk(MemorySourceKind::Folder, "/p"))
        .unwrap()
        .unwrap();
    assert_eq!(folder.path.as_deref(), Some("/p"));
    let file = reader_entry(&mk(MemorySourceKind::File, "/p/a.md"))
        .unwrap()
        .unwrap();
    assert_eq!(file.path.as_deref(), Some("/p/a.md"));
    for (kind, target) in [
        (MemorySourceKind::Link, "https://example.com/"),
        (MemorySourceKind::Github, "https://github.com/o/r"),
        (MemorySourceKind::Rss, "https://example.com/feed"),
    ] {
        let entry = reader_entry(&mk(kind, target)).unwrap().unwrap();
        assert_eq!(entry.url.as_deref(), Some(target));
    }
    assert!(reader_entry(&mk(MemorySourceKind::Composio, "gmail"))
        .unwrap()
        .is_none());
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
    assert_eq!(off.code(), crate::memory::error::MEMORY_OFF);

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

    let started = start_sync(&config, Some("src-ok")).unwrap();
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

    start_sync(&config, None).unwrap();
    let done = wait_for_idle(&config, "src-bad").await;
    assert_eq!(done.status, SourceStatus::Error);
    assert!(done.error.as_deref().is_some_and(|e| !e.is_empty()));
    assert!(done.last_sync_at.is_some());
}

#[test]
fn start_sync_refuses_unknown_ids_and_memory_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(
        start_sync(&config, None).unwrap_err().code(),
        crate::memory::error::MEMORY_OFF
    );
    bind_reference(&config);
    assert_eq!(
        start_sync(&config, Some("src-nope")).unwrap_err().code(),
        INVALID_REQUEST
    );
    assert!(
        start_sync(&config, None).unwrap().is_empty(),
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
    let key = (config.workspace_dir.clone(), "src-once".to_string());
    RUNNING.lock().unwrap().insert(key.clone());
    assert!(start_sync(&config, Some("src-once")).unwrap().is_empty());
    RUNNING.lock().unwrap().remove(&key);
    assert_eq!(start_sync(&config, Some("src-once")).unwrap().len(), 1);
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

    let started = sync_due(&config, now);
    assert_eq!(started, vec!["src-sched".to_string()]);
    wait_for_idle(&config, "src-sched").await;

    // Fifteen minutes later everything scheduled is due again; manual never is.
    let later = sync_due(&config, now + chrono::Duration::minutes(16));
    assert!(later.contains(&"src-sched".to_string()));
    assert!(later.contains(&"src-fresh".to_string()));
    assert!(!later.contains(&"src-manual".to_string()));
    wait_for_idle(&config, "src-sched").await;
    wait_for_idle(&config, "src-fresh").await;
}

#[test]
fn sync_due_with_memory_off_starts_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config
        .memory
        .sources
        .push(folder_source("s", tmp.path(), Some(15)));
    assert!(sync_due(&config, Utc::now()).is_empty());
}

#[tokio::test]
async fn store_all_skips_a_bad_item_but_fails_when_nothing_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let bound = engine::resolve(&config).engine().unwrap();
    let good = tinymemory::StoreItem::learning(
        "a good one",
        tinymemory::LearningKind::Fact,
        0.8,
        tinymemory::MemoryMeta::default(),
    );
    let bad = tinymemory::StoreItem::learning(
        "   ",
        tinymemory::LearningKind::Fact,
        0.8,
        tinymemory::MemoryMeta::default(),
    );
    assert_eq!(
        store_all(&bound, vec![bad.clone(), good], "src")
            .await
            .unwrap(),
        1
    );
    assert!(store_all(&bound, vec![bad], "src").await.is_err());
    assert_eq!(store_all(&bound, Vec::new(), "src").await.unwrap(), 0);
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
