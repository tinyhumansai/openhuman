use super::*;
use std::sync::{Arc, Mutex, OnceLock};

use crate::core::bus::BUS;

fn test_mutex() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[derive(Clone, Default)]
struct StageCollector {
    events: Arc<Mutex<Vec<DomainEvent>>>,
}

#[async_trait]
impl EventHandler<DomainEvent> for StageCollector {
    fn name(&self) -> &str {
        "memory::sync::tests::stage_collector"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if matches!(event, DomainEvent::MemorySyncStageChanged { .. }) {
            self.events.lock().unwrap().push(event.clone());
        }
    }
}

#[tokio::test]
async fn document_canonicalized_emits_stored_and_queued_stages() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    bridge
        .handle(&DomainEvent::DocumentCanonicalized {
            source_id: "slack:workspace-1".into(),
            source_kind: "chat".into(),
            chunks_written: 3,
            chunk_ids: vec!["chunk-1".into()],
            canonicalized_at: 1_700_000_000.0,
            body_preview: None,
        })
        .await;

    tokio::task::yield_now().await;

    let stages: Vec<String> = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            DomainEvent::MemorySyncStageChanged { stage, .. } => Some(stage.clone()),
            _ => None,
        })
        .collect();
    assert!(stages.contains(&"stored".to_string()));
    assert!(stages.contains(&"queued".to_string()));
}

#[tokio::test]
async fn memory_ingestion_started_emits_ingesting_stage() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    bridge
        .handle(&DomainEvent::MemoryIngestionStarted {
            document_id: "doc-123".into(),
            title: "Vault Note".into(),
            namespace: "vault:v-1".into(),
            queue_depth: 2,
        })
        .await;

    tokio::task::yield_now().await;

    let ingesting = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            DomainEvent::MemorySyncStageChanged {
                stage,
                provider,
                connection_id,
                detail,
                ..
            } if stage == "ingesting" => {
                Some((provider.clone(), connection_id.clone(), detail.clone()))
            }
            _ => None,
        })
        .expect("ingesting stage should be emitted");

    assert_eq!(ingesting.0.as_deref(), Some("vault:v-1"));
    assert_eq!(ingesting.1.as_deref(), Some("doc-123"));
    assert_eq!(ingesting.2.as_deref(), Some("queue_depth=2"));
}

// ── extract_mem_src_id tests ──────────────────────────────────────────

#[test]
fn extract_mem_src_id_parses_simple_source() {
    // "mem_src:<source_id>:<item_id>" → source_id
    assert_eq!(
        extract_mem_src_id("mem_src:src-abc-123:item-1"),
        Some("src-abc-123")
    );
}

#[test]
fn extract_mem_src_id_parses_item_id_with_colons_in_it() {
    // item_id may contain colons (e.g. RSS GUIDs that are URLs).
    // source_id is the first segment after "mem_src:"; item_id is everything after.
    assert_eq!(
        extract_mem_src_id("mem_src:src-rss-42:https://example.com/feed/item-7"),
        Some("src-rss-42")
    );
    // Web-page item ids may also contain colons.
    assert_eq!(
        extract_mem_src_id("mem_src:src-web-99:https://blog.example.com/2024/post"),
        Some("src-web-99")
    );
}

#[test]
fn extract_mem_src_id_returns_none_for_non_mem_src() {
    // Channel-provider syncs like "slack:workspace-1" have no mem_src prefix.
    assert_eq!(extract_mem_src_id("slack:workspace-1"), None);
    assert_eq!(extract_mem_src_id("gmail:alice-thread-1"), None);
    assert_eq!(extract_mem_src_id("no-prefix"), None);
}

#[test]
fn extract_mem_src_id_returns_none_for_missing_item_id() {
    // "mem_src:<source_id>" with no item_id separator is invalid.
    assert_eq!(extract_mem_src_id("mem_src:source-only-no-item"), None);
    // "mem_src:<source_id>:" with empty item_id is also invalid.
    assert_eq!(extract_mem_src_id("mem_src:src-abc:"), None);
}

// ── bridge populates source_id for Stored/Queued (DocumentCanonicalized) ──

#[tokio::test]
async fn bridge_populates_source_id_for_stored_and_queued_from_mem_src() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    bridge
        .handle(&DomainEvent::DocumentCanonicalized {
            // composite source_id format: mem_src:<source_id>:<item_id>
            source_id: "mem_src:src-folder-1:file-readme".into(),
            source_kind: "folder".into(),
            chunks_written: 2,
            chunk_ids: vec!["chunk-a".into()],
            canonicalized_at: 1_700_000_000.0,
            body_preview: None,
        })
        .await;

    tokio::task::yield_now().await;

    let source_ids: Vec<Option<String>> = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            DomainEvent::MemorySyncStageChanged {
                stage, source_id, ..
            } if stage == "stored" || stage == "queued" => Some(source_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(source_ids.len(), 2, "expected stored + queued events");
    for sid in &source_ids {
        assert_eq!(
            sid.as_deref(),
            Some("src-folder-1"),
            "[memory-sync] source_id should be extracted from mem_src prefix"
        );
    }
}

#[tokio::test]
async fn bridge_source_id_is_none_for_non_mem_src_canonicalized() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    // Non-memory-source sync (e.g. Slack channel sync) should have source_id=None
    bridge
        .handle(&DomainEvent::DocumentCanonicalized {
            source_id: "slack:workspace-1".into(),
            source_kind: "chat".into(),
            chunks_written: 5,
            chunk_ids: vec!["chunk-b".into()],
            canonicalized_at: 1_700_000_000.0,
            body_preview: None,
        })
        .await;

    tokio::task::yield_now().await;

    let source_ids: Vec<Option<String>> = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            DomainEvent::MemorySyncStageChanged {
                stage, source_id, ..
            } if stage == "stored" || stage == "queued" => Some(source_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(source_ids.len(), 2, "expected stored + queued events");
    for sid in &source_ids {
        assert!(
            sid.is_none(),
            "[memory-sync] source_id should be None for non-memory-source syncs"
        );
    }
}

#[tokio::test]
async fn bridge_populates_source_id_for_ingesting_from_mem_src() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    bridge
        .handle(&DomainEvent::MemoryIngestionStarted {
            document_id: "mem_src:src-rss-42:https://example.com/feed/item-7".into(),
            title: "Feed Item".into(),
            namespace: "user".into(),
            queue_depth: 1,
        })
        .await;

    tokio::task::yield_now().await;

    let ingesting = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            DomainEvent::MemorySyncStageChanged {
                stage,
                connection_id,
                source_id,
                ..
            } if stage == "ingesting" => Some((connection_id.clone(), source_id.clone())),
            _ => None,
        })
        .expect("ingesting stage should be emitted");

    // connection_id must still carry the full document_id (unchanged)
    assert_eq!(
        ingesting.0.as_deref(),
        Some("mem_src:src-rss-42:https://example.com/feed/item-7"),
        "[memory-sync] connection_id must carry original document_id unchanged"
    );
    // source_id extracts just the memory-source id
    assert_eq!(
        ingesting.1.as_deref(),
        Some("src-rss-42"),
        "[memory-sync] source_id should be extracted from document_id mem_src prefix"
    );
}

#[tokio::test]
async fn bridge_source_id_is_none_for_ingesting_non_mem_src() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::core::bus::init().await.expect("bus init");
    // The bridge emits through `tinymemory_api::events`' sink, and these tests
    // used to get it installed as a side effect of the memory host seams. Those
    // are gone with the in-process engine (#6161), so install the one seam this
    // actually needs — it is the host's own bus sink and names no engine.
    crate::openhuman::memory::host::install_memory_event_sink();

    let collector = StageCollector::default();
    let _subscription = BUS
        .subscribe(Arc::new(collector.clone()))
        .expect("event bus initialized");

    let bridge = MemorySyncStageBridge;
    // Non-memory-source ingestion (plain document_id, no mem_src prefix)
    bridge
        .handle(&DomainEvent::MemoryIngestionStarted {
            document_id: "doc-plain-uuid".into(),
            title: "Vault Note".into(),
            namespace: "vault:v-1".into(),
            queue_depth: 3,
        })
        .await;

    tokio::task::yield_now().await;

    let ingesting = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            DomainEvent::MemorySyncStageChanged {
                stage, source_id, ..
            } if stage == "ingesting" => Some(source_id.clone()),
            _ => None,
        })
        .expect("ingesting stage should be emitted");

    assert!(
        ingesting.is_none(),
        "[memory-sync] source_id should be None for non-mem_src document_id"
    );
}
