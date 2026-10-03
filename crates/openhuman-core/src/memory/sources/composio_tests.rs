use super::*;
use crate::config::schema::MemorySourceKind;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use tinymemory::{ItemKind, MetaFilter};

fn record(id: &str, title: &str, content: &str) -> ConnectorRecord {
    ConnectorRecord {
        item_id: id.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        ..ConnectorRecord::default()
    }
}

#[test]
fn record_item_builds_a_tagged_document() {
    let mut rec = record("m-1", "  Quarterly plan ", "ship memory v2");
    rec.mime = Some("text/plain".into());
    rec.url = Some("https://mail.example/m-1".into());
    rec.updated_at_ms = Some(1_700_000_000_000);
    rec.tags = vec!["inbox".into(), " ".into(), "gmail".into(), "inbox".into()];

    let item = record_item("GMail", "conn-7", "src-g", &rec).expect("an item");
    let StoreItem::Document {
        title,
        body,
        mime,
        meta,
    } = item
    else {
        panic!("expected a document");
    };
    assert_eq!(title.as_deref(), Some("Quarterly plan"));
    assert!(matches!(body, DocumentBody::Text(ref t) if t == "ship memory v2"));
    assert_eq!(mime.as_deref(), Some("text/plain"));
    assert_eq!(meta.url.as_deref(), Some("https://mail.example/m-1"));
    assert_eq!(meta.source.kind, SourceKind::Composio);
    assert_eq!(meta.source.id.as_deref(), Some("src-g"));
    assert_eq!(
        meta.tags,
        vec![
            "gmail".to_string(),
            "connection:conn-7".to_string(),
            "inbox".to_string()
        ],
        "toolkit and connection first, blanks and duplicates dropped"
    );
    assert_eq!(
        meta.observed_at.map(|t| t.timestamp_millis()),
        Some(1_700_000_000_000)
    );
}

#[test]
fn record_item_skips_empty_content_and_blank_titles() {
    assert!(record_item("gmail", "c", "s", &record("1", "t", "   ")).is_none());
    let StoreItem::Document { title, .. } =
        record_item("gmail", "c", "s", &record("1", "  ", "body")).unwrap()
    else {
        panic!("expected a document");
    };
    assert!(title.is_none());
}

#[test]
fn connection_tag_is_namespaced() {
    assert_eq!(connection_tag("abc"), "connection:abc");
}

#[tokio::test]
async fn store_records_stores_the_non_empty_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let bound = crate::memory::engine::resolve(&config).engine().unwrap();
    let records = vec![
        record("1", "One", "first record"),
        record("2", "Empty", " "),
        record("3", "Three", "third record"),
    ];
    let stored_count = store_records(&bound, "notion", "conn-1", "src-n", &records)
        .await
        .unwrap();
    assert_eq!(stored_count, 2);
    let docs = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Document],
            ..MetaFilter::default()
        },
    )
    .await;
    assert_eq!(docs.len(), 2);
    assert!(docs
        .iter()
        .all(|d| d.meta.source.kind == SourceKind::Composio));
}

#[tokio::test]
async fn store_records_with_nothing_to_store_is_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let bound = crate::memory::engine::resolve(&config).engine().unwrap();
    assert_eq!(
        store_records(&bound, "notion", "c", "s", &[])
            .await
            .unwrap(),
        0
    );
}

#[test]
fn source_id_for_toolkit_prefers_the_configured_source() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    assert_eq!(source_id_for_toolkit(&config, "Gmail"), "composio:gmail");
    config
        .memory
        .sources
        .push(crate::config::schema::MemorySourceConfig {
            id: "src-gmail".into(),
            kind: MemorySourceKind::Composio,
            target: "gmail".into(),
            label: "Gmail".into(),
            schedule_mins: None,
        });
    config
        .memory
        .sources
        .push(crate::config::schema::MemorySourceConfig {
            id: "src-folder".into(),
            kind: MemorySourceKind::Folder,
            target: "notion".into(),
            label: "Folder named like a toolkit".into(),
            schedule_mins: None,
        });
    assert_eq!(source_id_for_toolkit(&config, "GMAIL"), "src-gmail");
    assert_eq!(source_id_for_toolkit(&config, "notion"), "composio:notion");
}

#[tokio::test]
async fn forget_connection_removes_only_that_connections_items() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let bound = crate::memory::engine::resolve(&config).engine().unwrap();
    store_records(
        &bound,
        "gmail",
        "conn-a",
        "src",
        &[record("1", "A", "from a")],
    )
    .await
    .unwrap();
    store_records(
        &bound,
        "gmail",
        "conn-b",
        "src",
        &[record("2", "B", "from b")],
    )
    .await
    .unwrap();
    assert_eq!(forget_connection(&config, "conn-a").await.unwrap(), 1);
    let left = stored(&engine, MetaFilter::default()).await;
    assert_eq!(left.len(), 1);
    assert!(left[0].meta.tags.contains(&"connection:conn-b".to_string()));
}

#[tokio::test]
async fn forget_connection_with_memory_off_forgets_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(forget_connection(&config, "conn-a").await.unwrap(), 0);
}

#[tokio::test]
async fn sync_toolkit_without_a_connector_is_an_error_not_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let bound = crate::memory::engine::resolve(&config).engine().unwrap();
    let source = crate::config::schema::MemorySourceConfig {
        id: "src-gmail".into(),
        kind: MemorySourceKind::Composio,
        target: "gmail".into(),
        label: "Gmail".into(),
        schedule_mins: None,
    };
    assert!(sync_toolkit(&config, &bound, &source).await.is_err());
}
