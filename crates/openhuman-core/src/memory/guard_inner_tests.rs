use super::*;
use async_trait::async_trait;
use tinymemory_api::*;

use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{MemoryMeta, MetaFilter};

const SECRET: &str = concat!(
    "sk-ant-api03-",
    "AAAAAAAAAAAAAAAAAAAAAAAA",
    "AAAAAAAAAAAAAAAAAAAAAAAA",
);

async fn texts(engine: &ReferenceEngine) -> Vec<String> {
    engine
        .list(ListRequest {
            filter: MetaFilter::default(),
            limit: 50,
            cursor: None,
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|hit| hit.text)
        .collect()
}

#[tokio::test]
async fn every_write_path_is_scrubbed() {
    let reference = Arc::new(ReferenceEngine::new());
    let guarded = wrap(reference.clone());
    let item =
        |text: &str| StoreItem::document(format!("{text} key={SECRET}"), MemoryMeta::default());

    guarded.store(item("one")).await.unwrap();
    guarded
        .store_with(item("two"), WriteOptions::accepted())
        .await
        .unwrap();
    guarded
        .store_many(vec![item("three"), item("four")])
        .await
        .unwrap();
    guarded
        .store_many_with(vec![item("five"), item("six")], WriteOptions::accepted())
        .await
        .unwrap();

    let stored = texts(&reference).await;
    assert_eq!(stored.len(), 6);
    assert!(
        stored.iter().all(|text| !text.contains(SECRET)),
        "a secret reached the engine: {stored:?}"
    );
    assert_eq!(guarded.descriptor().id, reference.descriptor().id);
}

#[tokio::test]
async fn every_read_passes_through_the_timer_unchanged() {
    let reference = Arc::new(ReferenceEngine::new());
    let guarded = wrap(reference.clone());
    let id = guarded
        .store(StoreItem::document("oolong tea", MemoryMeta::default()))
        .await
        .unwrap()
        .id;
    let reach = tinymemory_api::Reach::subtree(tinymemory_api::Namespace::ROOT);

    assert_eq!(guarded.health().await, reference.health().await);
    assert_eq!(
        guarded
            .recall(RecallRequest::new("oolong", 5))
            .await
            .unwrap(),
        reference
            .recall(RecallRequest::new("oolong", 5))
            .await
            .unwrap()
    );
    let fetch = || FetchRequest::new("oolong", tinymemory_api::FetchMode::Keyword, 5);
    assert_eq!(
        guarded.fetch(fetch()).await.unwrap(),
        reference.fetch(fetch()).await.unwrap()
    );
    let get = || GetRequest {
        ids: vec![id.clone()],
        reach: None,
    };
    assert_eq!(guarded.get(get()).await.unwrap().len(), 1);
    let explore = || ExploreRequest::new(tinymemory_api::Facet::Kind, 5);
    assert_eq!(
        guarded.explore(explore()).await.ok(),
        reference.explore(explore()).await.ok()
    );
    assert_eq!(
        guarded
            .consolidate(ConsolidateRequest::new(reach.clone()))
            .await
            .ok(),
        reference
            .consolidate(ConsolidateRequest::new(reach.clone()))
            .await
            .ok()
    );
    assert_eq!(
        guarded
            .beliefs(BeliefsRequest::new(reach.clone(), 5))
            .await
            .ok(),
        reference.beliefs(BeliefsRequest::new(reach, 5)).await.ok()
    );
    assert_eq!(
        guarded
            .forget(ForgetTarget::Ids(vec![id]))
            .await
            .unwrap()
            .forgotten,
        1
    );
}

#[test]
fn scrubbing_keeps_the_page_breaks_a_document_is_chunked_by() {
    let body = format!("Page one key={SECRET}\u{c}Page two.\u{c}Page three.");
    let scrubbed = scrub(StoreItem::document(body, MemoryMeta::default()));
    let StoreItem::Document {
        body: tinymemory_api::DocumentBody::Text(text),
        ..
    } = scrubbed
    else {
        panic!("a document");
    };
    assert!(!text.contains(SECRET), "{text}");
    assert_eq!(text.matches('\u{c}').count(), 2, "{text:?}");
    assert!(text.ends_with("\u{c}Page two.\u{c}Page three."), "{text:?}");
}

#[tokio::test]
async fn export_and_erase_reach_the_wrapped_engine() {
    let reference = Arc::new(ReferenceEngine::new());
    let guarded = wrap(reference.clone());
    let namespace: tinymemory_api::Namespace = "agent:a".parse().unwrap();
    guarded
        .store(StoreItem::document(
            "kept whole",
            MemoryMeta {
                namespace: namespace.clone(),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();
    let page = guarded
        .export(ListRequest::new(MetaFilter::default(), 10))
        .await
        .expect("export is forwarded, not refused by the wrapper");
    assert_eq!(page.items.len(), 1);
    let report = guarded
        .erase(EraseRequest::new(tinymemory_api::Reach::exact(namespace)))
        .await
        .expect("erase is forwarded, not refused by the wrapper");
    assert_eq!(report.erased_scopes, 1);
    assert!(texts(&reference).await.is_empty());
}

/// Records the wait each bulk store asks for, storing through a reference
/// engine.
#[derive(Default)]
struct Recording {
    inner: ReferenceEngine,
    waits: std::sync::Mutex<Vec<WaitFor>>,
    reaches: std::sync::Mutex<Vec<Reach>>,
}

#[async_trait::async_trait]
impl MemoryEngine for Recording {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(&self, req: RecallRequest) -> Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(req).await
    }
    async fn fetch(&self, req: FetchRequest) -> Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }
    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        self.inner.store(item).await
    }
    async fn store_many_with(
        &self,
        items: Vec<StoreItem>,
        options: WriteOptions,
    ) -> Result<Vec<StoreReceipt>> {
        self.waits.lock().unwrap().push(options.wait);
        self.inner.store_many(items).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn forget_within(&self, ids: Vec<ItemId>, reach: Reach) -> Result<ForgetReport> {
        self.reaches.lock().unwrap().push(reach.clone());
        self.inner.forget_within(ids, reach).await
    }
    async fn list(&self, req: ListRequest) -> Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
}

#[tokio::test]
async fn forget_within_reaches_the_wrapped_engine_with_its_reach() {
    let recording = Arc::new(Recording::default());
    let guarded = wrap(recording.clone());
    let reach = Reach::subtree("agent:ann".parse::<tinymemory_api::Namespace>().unwrap());
    guarded
        .forget_within(vec![ItemId::new("a")], reach.clone())
        .await
        .unwrap();
    assert_eq!(
        *recording.reaches.lock().unwrap(),
        vec![reach],
        "the guard forwards forget_within instead of the default's forget by id"
    );
}

#[tokio::test]
async fn a_bulk_store_keeps_its_wait_through_the_guard() {
    let recording = Arc::new(Recording::default());
    let guarded = wrap(recording.clone());
    let items = || vec![StoreItem::document("tea", MemoryMeta::default())];
    guarded
        .store_many_with(items(), WriteOptions::accepted())
        .await
        .unwrap();
    guarded
        .store_many_with(items(), WriteOptions::visible())
        .await
        .unwrap();
    assert_eq!(
        *recording.waits.lock().unwrap(),
        vec![WaitFor::Accepted, WaitFor::Visible],
        "the guard passes the wait on instead of serving it as store_many"
    );
}

/// The reference engine, with a preview listing that says it is one, so a
/// wrapper that fell back to `list` would show.
struct MarkedPreview(ReferenceEngine);

#[async_trait]
impl MemoryEngine for MarkedPreview {
    fn descriptor(&self) -> &EngineDescriptor {
        self.0.descriptor()
    }
    async fn health(&self) -> EngineHealth {
        self.0.health().await
    }
    async fn recall(&self, req: RecallRequest) -> Result<RecallAnswer> {
        self.0.recall(req).await
    }
    async fn fetch(&self, req: FetchRequest) -> Result<FetchPage> {
        self.0.fetch(req).await
    }
    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        self.0.store(item).await
    }
    async fn forget(&self, target: ForgetTarget) -> Result<ForgetReport> {
        self.0.forget(target).await
    }
    async fn list(&self, req: ListRequest) -> Result<ListPage> {
        self.0.list(req).await
    }
    async fn list_preview(&self, req: ListRequest) -> Result<ListPage> {
        let mut page = self.0.list(req).await?;
        for hit in &mut page.items {
            hit.text = format!("preview: {}", hit.text);
        }
        Ok(page)
    }
}

#[tokio::test]
async fn a_preview_listing_reaches_the_wrapped_engine() {
    let guarded = wrap(Arc::new(MarkedPreview(ReferenceEngine::new())));
    guarded
        .store(StoreItem::document("oolong tea", MemoryMeta::default()))
        .await
        .unwrap();
    let req = || ListRequest {
        filter: MetaFilter::default(),
        limit: 5,
        cursor: None,
    };
    let preview = guarded.list_preview(req()).await.unwrap();
    assert_eq!(preview.items.len(), 1);
    assert!(
        preview.items[0].text.starts_with("preview: "),
        "{}",
        preview.items[0].text
    );
    let whole = guarded.list(req()).await.unwrap();
    assert!(!whole.items[0].text.starts_with("preview: "));
}
