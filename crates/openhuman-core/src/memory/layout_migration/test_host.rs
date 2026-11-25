//! A host over two in-memory engines, for the migration's tests.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    LearningKind, ListRequest, MemoryEngine, MemoryMeta, MetaFilter, Namespace, StoreItem,
};
use tinymemory_tools::layout_migration::{
    Engines, FlowPlacement, FsLegacyClaim, LayoutHost, LegacyClaim, MigrationStateStore, Placement,
    Result,
};
use tinymemory_tools::MemoryLayout;

use super::state::StateKvMigrationStore;
use crate::config::Config;

/// Two in-memory engines and the switches a real host keeps, with the real
/// state KV value of the config's workspace.
pub(crate) struct FakeHost {
    pub(crate) legacy: Arc<ReferenceEngine>,
    pub(crate) tree: Arc<ReferenceEngine>,
    pub(crate) switched: AtomicBool,
    pub(crate) free: AtomicBool,
    /// The claim when the legacy tree is shared.
    pub(crate) claim: Option<FsLegacyClaim>,
    /// Asks for free_now: after this many, moving stops being free.
    pub(crate) free_for: AtomicUsize,
    /// Written to the legacy tree as the switch happens (a turn racing it).
    pub(crate) racing_write: Mutex<Option<StoreItem>>,
    store: StateKvMigrationStore,
    workspace: std::path::PathBuf,
}

impl FakeHost {
    pub(crate) async fn with(learnings: usize, config: &Config) -> Self {
        let legacy = Arc::new(ReferenceEngine::new());
        for i in 0..learnings {
            legacy.store(fact(&format!("fact {i}"))).await.unwrap();
        }
        Self {
            legacy,
            tree: Arc::new(ReferenceEngine::new()),
            switched: AtomicBool::new(false),
            free: AtomicBool::new(true),
            claim: None,
            free_for: AtomicUsize::new(usize::MAX),
            racing_write: Mutex::new(None),
            store: StateKvMigrationStore::new(&config.workspace_dir),
            workspace: config.workspace_dir.clone(),
        }
    }
}

pub(crate) fn fact(text: &str) -> StoreItem {
    StoreItem::learning(text, LearningKind::Fact, 0.5, MemoryMeta::default())
}

pub(crate) async fn count(engine: &dyn MemoryEngine) -> usize {
    let mut total = 0;
    let mut cursor = None;
    loop {
        let mut request = ListRequest::new(MetaFilter::default(), 100);
        request.cursor = cursor;
        let page = engine.list(request).await.unwrap();
        total += page.items.len();
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return total,
        }
    }
}

#[async_trait::async_trait]
impl LayoutHost for FakeHost {
    fn store(&self) -> &dyn MigrationStateStore {
        &self.store
    }

    async fn engines(&self) -> Result<Engines> {
        Ok(Engines {
            legacy: self.legacy.clone(),
            tree: self.tree.clone(),
        })
    }

    fn placement(&self) -> Result<Placement> {
        Ok(Placement {
            layout: MemoryLayout::new(Namespace::ROOT).unwrap(),
            chat_node: "ws:main".parse().unwrap(),
            flows: FlowPlacement::WithRoot,
            split_github_by_repo: false,
        })
    }

    fn is_switched(&self) -> bool {
        self.switched.load(Ordering::SeqCst)
    }

    async fn switch(&self) -> Result<()> {
        let racing = self.racing_write.lock().unwrap().take();
        if let Some(item) = racing {
            self.legacy.store(item).await.unwrap();
        }
        self.switched.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn free_now(&self) -> bool {
        let left = self.free_for.fetch_sub(1, Ordering::SeqCst);
        self.free.load(Ordering::SeqCst) && left > 0
    }

    async fn legacy_claim(&self) -> Result<Option<Box<dyn LegacyClaim>>> {
        Ok(self
            .claim
            .clone()
            .map(|key| Box::new(key) as Box<dyn LegacyClaim>))
    }

    fn listed_unconfirmed(&self) -> bool {
        crate::memory::import::listed_unconfirmed(&self.workspace)
    }

    async fn wait_for_listing(&self) {}
}
