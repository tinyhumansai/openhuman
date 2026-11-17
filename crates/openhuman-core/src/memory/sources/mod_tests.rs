use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use chrono::Utc;
use tinymemory_api::{DocumentBody, MemoryMeta, SourceKind, SourceRef, StoreItem};
use tinymemory_integrations::sources::sync::MIN_SCHEDULE_MINS;
use tinymemory_integrations::sources::{SourceState, SourceStatus};

fn add_params(kind: &str, target: &str) -> SourcesAddParams {
    SourcesAddParams {
        kind: kind.to_string(),
        target: target.to_string(),
        label: None,
        schedule_mins: None,
        namespace: None,
    }
}

fn source(id: &str, mins: Option<u32>) -> MemorySourceConfig {
    MemorySourceConfig {
        id: id.to_string(),
        kind: MemorySourceKind::Folder,
        target: "/tmp/x".to_string(),
        label: id.to_string(),
        schedule_mins: mins,
        namespace: None,
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
}

#[test]
fn normalize_target_rejects_malformed_targets() {
    for (kind, target, expected) in [
        (MemorySourceKind::Folder, "   ", "target must not be empty"),
        (
            MemorySourceKind::Github,
            "just-one-part",
            "a GitHub target is `owner/repo` or a repository URL",
        ),
        (
            MemorySourceKind::Github,
            "a/b/c",
            "a GitHub target is `owner/repo` or a repository URL",
        ),
        (
            MemorySourceKind::Github,
            "/repo",
            "a GitHub target is `owner/repo` or a repository URL",
        ),
        (MemorySourceKind::Link, "not a url", "target is not a URL"),
        (
            MemorySourceKind::Link,
            "ftp://example.com/x",
            "target must be an http(s) URL",
        ),
        (
            MemorySourceKind::Rss,
            "file:///etc/passwd",
            "target must be an http(s) URL",
        ),
    ] {
        match normalize_target(kind, target).unwrap_err() {
            tinymemory_api::Error::InvalidRequest(message) => assert_eq!(message, expected),
            error => panic!("{kind:?} {target} returned unexpected error: {error:?}"),
        }
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
            namespace: None,
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
    match duplicate {
        tinymemory_api::Error::InvalidRequest(message) => {
            assert_eq!(message, "that source is already added")
        }
        error => panic!("duplicate source returned unexpected error: {error:?}"),
    }

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
        "INVALID_REQUEST"
    );
    let schedule_error = apply_add(
        &mut config,
        &SourcesAddParams {
            schedule_mins: Some(MIN_SCHEDULE_MINS - 1),
            namespace: None,
            ..add_params("folder", "/p")
        },
    )
    .unwrap_err();
    match schedule_error {
        tinymemory_api::Error::InvalidRequest(message) => assert_eq!(
            message,
            format!("schedule_mins must be at least {MIN_SCHEDULE_MINS}")
        ),
        error => panic!("invalid schedule returned unexpected error: {error:?}"),
    }

    let target_error = apply_add(&mut config, &add_params("link", "nope")).unwrap_err();
    match target_error {
        tinymemory_api::Error::InvalidRequest(message) => {
            assert_eq!(message, "target is not a URL")
        }
        error => panic!("invalid target returned unexpected error: {error:?}"),
    }

    let mut namespace_params = add_params("folder", "/p");
    namespace_params.namespace = Some("bogus:root".into());
    let namespace_error = apply_add(&mut config, &namespace_params).unwrap_err();
    match namespace_error {
        tinymemory_api::Error::InvalidRequest(message) => {
            assert_eq!(
                message,
                "invalid request: `bogus` is not a namespace segment kind"
            )
        }
        error => panic!("invalid namespace returned unexpected error: {error:?}"),
    }
    assert!(
        config.memory.sources.is_empty(),
        "a refused add changes nothing"
    );
}

#[test]
fn the_removed_composio_kind_is_rejected_and_every_other_kind_is_accepted() {
    let mut config = Config::default();
    let error = apply_add(&mut config, &add_params("composio", "gmail")).unwrap_err();
    assert_eq!(error.code(), "INVALID_REQUEST");
    assert!(config.memory.sources.is_empty());
    for (kind, target) in [
        ("folder", "/notes"),
        ("file", "/notes/a.md"),
        ("link", "https://example.com/docs"),
        ("github", "acme/widgets"),
        ("rss", "https://example.com/feed.xml"),
    ] {
        assert!(
            apply_add(&mut config, &add_params(kind, target)).is_ok(),
            "{kind}"
        );
    }
    assert_eq!(config.memory.sources.len(), 5);
}

#[test]
fn view_overlays_the_sync_state() {
    let src = source("src-1", Some(30));
    let state = SourceState {
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
    let left = stored(&engine, tinymemory_api::MetaFilter::default()).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].meta.source.id.as_deref(), Some("src-b"));
}

#[tokio::test]
async fn forget_items_with_memory_off_is_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(forget_items(&config, "src-a").await.unwrap(), 0);
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Source {
            source_id: "src-a".into()
        }],
        "queued for the next sign-in, not dropped"
    );
}

#[tokio::test]
async fn forget_items_queues_a_failed_source_deletion_and_returns_the_engine_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Engine(
        "source forget failed".into(),
    ))
    .bind(&config);

    let error = forget_items(&config, "src-a")
        .await
        .expect_err("a reachable engine failure remains visible to the caller");
    assert_eq!(error.code(), "ENGINE");
    assert!(error.to_string().contains("source forget failed"));
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Source {
            source_id: "src-a".into()
        }],
        "the deletion stays queued for a later retry"
    );
}

#[test]
fn a_source_can_store_at_an_agent_node() {
    let mut config = Config::default();
    let mut params = add_params("link", "https://example.com/a");
    params.namespace = Some("team:acme".into());
    let added = apply_add(&mut config, &params).unwrap();
    assert_eq!(added.namespace.as_deref(), Some("team:acme"));
    assert_eq!(
        layout_of(&config, &added.id).root().to_string(),
        "team:acme"
    );
    assert!(layout_of(&config, "unknown").root().is_root());

    let mut bad = add_params("link", "https://example.com/b");
    bad.namespace = Some("nope".into());
    assert_eq!(
        apply_add(&mut config, &bad).unwrap_err().code(),
        "INVALID_REQUEST"
    );

    let mut hand_edited = source("x", None);
    hand_edited.namespace = Some("not a node".into());
    assert!(layout_of_source(&config, &hand_edited).root().is_root());
}
