use super::*;
use crate::memory::error::INVALID_REQUEST;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use chrono::Duration;
use tinymemory::{DocumentBody, MemoryMeta, SourceKind, SourceRef, StoreItem};

fn add_params(kind: &str, target: &str) -> SourcesAddParams {
    SourcesAddParams {
        kind: kind.to_string(),
        target: target.to_string(),
        label: None,
        schedule_mins: None,
    }
}

fn source(id: &str, mins: Option<u32>) -> MemorySourceConfig {
    MemorySourceConfig {
        id: id.to_string(),
        kind: MemorySourceKind::Folder,
        target: "/tmp/x".to_string(),
        label: id.to_string(),
        schedule_mins: mins,
    }
}

#[test]
fn normalize_target_accepts_each_kinds_shape() {
    assert_eq!(
        normalize_target(MemorySourceKind::Folder, "  /home/me/notes ").unwrap(),
        "/home/me/notes"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::File, "notes.md").unwrap(),
        "notes.md"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::Github, "owner/repo").unwrap(),
        "https://github.com/owner/repo"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::Github, "https://github.com/o/r").unwrap(),
        "https://github.com/o/r"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::Rss, "https://example.com/feed.xml").unwrap(),
        "https://example.com/feed.xml"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::Link, "https://example.com").unwrap(),
        "https://example.com/"
    );
    assert_eq!(
        normalize_target(MemorySourceKind::Composio, "GMail").unwrap(),
        "gmail"
    );
}

#[test]
fn normalize_target_rejects_malformed_targets() {
    for (kind, target) in [
        (MemorySourceKind::Folder, "   "),
        (MemorySourceKind::Github, "just-one-part"),
        (MemorySourceKind::Github, "a/b/c"),
        (MemorySourceKind::Github, "/repo"),
        (MemorySourceKind::Link, "not a url"),
        (MemorySourceKind::Link, "ftp://example.com/x"),
        (MemorySourceKind::Rss, "file:///etc/passwd"),
        (MemorySourceKind::Composio, "has space"),
        (MemorySourceKind::Composio, "a/b"),
    ] {
        assert_eq!(
            normalize_target(kind, target).unwrap_err().code(),
            INVALID_REQUEST,
            "{kind:?} {target}"
        );
    }
}

#[test]
fn add_list_and_remove_round_trip_through_config() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);

    let added = apply_add(
        &mut config,
        &SourcesAddParams {
            label: Some("  My notes ".into()),
            schedule_mins: Some(60),
            ..add_params("Folder", "/home/me/notes")
        },
    )
    .unwrap();
    assert!(added.id.starts_with("src-"));
    assert_eq!(added.kind, MemorySourceKind::Folder);
    assert_eq!(added.label, "My notes");
    assert_eq!(added.schedule_mins, Some(60));

    let unlabeled = apply_add(&mut config, &add_params("github", "owner/repo")).unwrap();
    assert_eq!(
        unlabeled.label, "https://github.com/owner/repo",
        "label defaults to the target"
    );

    let listed = list(&config);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, added.id);
    assert_eq!(listed[0].status, SourceStatus::Idle);
    assert_eq!(listed[0].items, 0);
    assert!(listed[0].last_sync_at.is_none());

    // The same kind and target cannot be added twice.
    let duplicate = apply_add(&mut config, &add_params("folder", "/home/me/notes")).unwrap_err();
    assert_eq!(duplicate.code(), INVALID_REQUEST);

    let removed = apply_remove(&mut config, &added.id).expect("removed");
    assert_eq!(removed.id, added.id);
    assert_eq!(list(&config).len(), 1);
    assert!(apply_remove(&mut config, &added.id).is_none());
}

#[test]
fn add_validates_kind_schedule_and_target() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    assert_eq!(
        apply_add(&mut config, &add_params("twitter", "x"))
            .unwrap_err()
            .code(),
        INVALID_REQUEST
    );
    assert_eq!(
        apply_add(
            &mut config,
            &SourcesAddParams {
                schedule_mins: Some(MIN_SCHEDULE_MINS - 1),
                ..add_params("folder", "/p")
            }
        )
        .unwrap_err()
        .code(),
        INVALID_REQUEST
    );
    assert_eq!(
        apply_add(&mut config, &add_params("link", "nope"))
            .unwrap_err()
            .code(),
        INVALID_REQUEST
    );
    assert!(
        config.memory.sources.is_empty(),
        "a refused add changes nothing"
    );
    // A Composio source needs no reader.
    apply_add(&mut config, &add_params("composio", "Gmail")).unwrap();
    assert_eq!(config.memory.sources[0].target, "gmail");
}

#[test]
fn view_overlays_the_sync_state() {
    let src = source("src-1", Some(30));
    let state = state::SourceState {
        last_sync_at: Some(Utc::now()),
        status: SourceStatus::Error,
        error: Some("boom".into()),
        items: 7,
    };
    let view = view(&src, Some(&state));
    assert_eq!(view.status, SourceStatus::Error);
    assert_eq!(view.error.as_deref(), Some("boom"));
    assert_eq!(view.items, 7);
    assert_eq!(view.schedule_mins, Some(30));
    assert_eq!(super::view(&src, None).status, SourceStatus::Idle);
}

#[test]
fn is_due_follows_schedule_and_status() {
    let now = Utc::now();
    let scheduled = source("s", Some(60));
    assert!(
        !is_due(&source("manual", None), None, now),
        "on demand only"
    );
    assert!(is_due(&scheduled, None, now), "never synced");

    let synced = |ago_mins: i64, status| state::SourceState {
        last_sync_at: Some(now - Duration::minutes(ago_mins)),
        status,
        ..state::SourceState::default()
    };
    assert!(!is_due(
        &scheduled,
        Some(&synced(59, SourceStatus::Idle)),
        now
    ));
    assert!(is_due(
        &scheduled,
        Some(&synced(60, SourceStatus::Idle)),
        now
    ));
    assert!(
        is_due(&scheduled, Some(&synced(500, SourceStatus::Error)), now),
        "errors retry"
    );
    assert!(!is_due(
        &scheduled,
        Some(&synced(500, SourceStatus::Syncing)),
        now
    ));
    let no_last_sync = state::SourceState::default();
    assert!(is_due(&scheduled, Some(&no_last_sync), now));
}

#[test]
fn is_due_never_schedules_faster_than_the_minimum() {
    let now = Utc::now();
    let mut eager = source("s", Some(1));
    eager.schedule_mins = Some(1);
    let state = state::SourceState {
        last_sync_at: Some(now - Duration::minutes(5)),
        ..state::SourceState::default()
    };
    assert!(!is_due(&eager, Some(&state), now));
}

#[tokio::test]
async fn forget_items_removes_only_that_sources_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    for (source_id, text) in [("src-a", "alpha doc"), ("src-b", "beta doc")] {
        crate::memory::ops::store_item(
            &config,
            StoreItem::Document {
                title: Some(text.into()),
                body: DocumentBody::Text(text.into()),
                mime: None,
                meta: MemoryMeta {
                    source: SourceRef {
                        kind: SourceKind::Folder,
                        id: Some(source_id.into()),
                    },
                    ..MemoryMeta::default()
                },
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(forget_items(&config, "src-a").await.unwrap(), 1);
    let left = stored(&engine, tinymemory::MetaFilter::default()).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].meta.source.id.as_deref(), Some("src-b"));
}

#[tokio::test]
async fn forget_items_with_memory_off_is_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(forget_items(&config, "src-a").await.unwrap(), 0);
}
