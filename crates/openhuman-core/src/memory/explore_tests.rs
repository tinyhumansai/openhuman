use super::*;

use tinymemory_api::{ItemKind, MemoryEngine, MemoryMeta, SourceKind, SourceRef, StoreItem};

use crate::memory::ops;
use crate::memory::test_fixtures::{bind_reference, config_in};
use crate::memory::types::ItemsListParams;

fn step(facet: Facet, value: &str) -> PathStep {
    PathStep {
        facet,
        value: value.to_string(),
    }
}

fn params(facet: Facet, path: Vec<PathStep>) -> ExploreParams {
    ExploreParams {
        facet,
        path,
        filter: None,
        limit: None,
        scan_limit: None,
    }
}

fn doc(text: &str, folder: &str) -> StoreItem {
    let mut meta = MemoryMeta::from_source(SourceKind::Folder, Some("notes".into()));
    meta.folder = Some(folder.to_string());
    meta.file_path = Some(format!("{folder}/{text}.md"));
    StoreItem::document(text, meta)
}

async fn seed(engine: &dyn MemoryEngine) -> Vec<String> {
    let mut ids = Vec::new();
    for item in [
        doc("alpha", "/notes"),
        doc("beta", "/notes"),
        doc("gamma", "/notes/deep"),
        StoreItem::learning(
            "prefers tea",
            tinymemory_api::LearningKind::Preference,
            0.9,
            MemoryMeta {
                source: SourceRef {
                    kind: SourceKind::Agent,
                    id: None,
                },
                ..MemoryMeta::default()
            },
        ),
    ] {
        ids.push(engine.store(item).await.unwrap().id.as_str().to_string());
    }
    ids
}

#[tokio::test]
async fn explore_counts_and_drills_down_by_path() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    seed(engine.as_ref()).await;

    let kinds = explore(&config, params(Facet::Kind, Vec::new()))
        .await
        .unwrap();
    let counts: Vec<(&str, u64)> = kinds
        .buckets
        .iter()
        .map(|b| (b.value.as_str(), b.count))
        .collect();
    assert_eq!(counts, [("document", 3), ("learning", 1)]);
    assert_eq!(kinds.total, 4);

    let folders = explore(
        &config,
        params(Facet::Folder, vec![step(Facet::Kind, "document")]),
    )
    .await
    .unwrap();
    assert_eq!(folders.total, 3);
    assert_eq!(folders.buckets[0].value, "/notes");
    assert_eq!(folders.buckets[0].count, 2);

    // `/notes` narrows by prefix, so its subfolder's item lists too.
    let listed = ops::items_list(
        &config,
        ItemsListParams {
            path: vec![step(Facet::Kind, "document"), step(Facet::Folder, "/notes")],
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.items.len(), 3);
    let deeper = ops::items_list(
        &config,
        ItemsListParams {
            path: vec![step(Facet::FilePath, "/notes/deep")],
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(deeper.items.len(), 1);
    assert!(deeper.items[0].text.contains("gamma"));
}

#[tokio::test]
async fn the_path_applies_on_top_of_a_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    seed(engine.as_ref()).await;
    let page = explore(
        &config,
        ExploreParams {
            filter: Some(MetaFilter::kinds([ItemKind::Learning])),
            ..params(Facet::Source, vec![step(Facet::Source, "agent")])
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.buckets[0].value, "agent");
}

#[tokio::test]
async fn items_get_reads_items_whole_in_the_order_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let ids = seed(engine.as_ref()).await;

    let view = items_get(
        &config,
        ItemsGetParams {
            ids: vec![ids[3].clone(), "nope".into(), ids[0].clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    let got: Vec<&str> = view.items.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(got, [ids[3].as_str(), ids[0].as_str()]);
    assert!(view.items[0].text.contains("prefers tea"));
    assert_eq!(view.items[1].meta.folder.as_deref(), Some("/notes"));
}

#[tokio::test]
async fn bad_requests_are_invalid_and_memory_off_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(
        explore(&config, params(Facet::Kind, Vec::new()))
            .await
            .unwrap_err()
            .code(),
        "MEMORY_OFF"
    );
    assert_eq!(
        items_get(
            &config,
            ItemsGetParams {
                ids: vec!["x".into()],
                reach: None,
            }
        )
        .await
        .unwrap_err()
        .code(),
        "MEMORY_OFF"
    );

    bind_reference(&config);
    for bad in [
        params(Facet::Kind, vec![step(Facet::Kind, "memo")]),
        params(Facet::Kind, vec![step(Facet::Workspace, "  ")]),
        params(
            Facet::Kind,
            (0..=MAX_PATH_STEPS)
                .map(|i| step(Facet::Tag, &i.to_string()))
                .collect(),
        ),
        ExploreParams {
            limit: Some(0),
            ..params(Facet::Kind, Vec::new())
        },
    ] {
        assert_eq!(
            explore(&config, bad).await.unwrap_err().code(),
            "INVALID_REQUEST"
        );
    }
    assert_eq!(
        items_get(
            &config,
            ItemsGetParams {
                ids: Vec::new(),
                reach: None
            }
        )
        .await
        .unwrap_err()
        .code(),
        "INVALID_REQUEST"
    );
}
