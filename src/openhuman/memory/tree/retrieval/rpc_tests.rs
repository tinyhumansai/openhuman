//! Unit tests for the Phase 4 retrieval RPC handlers.
//!
//! Scope: the handler layer specifically — param parsing, default
//! fallbacks, `SourceKind` / `EntityKind` validation, the scope each call
//! forwards to the contract, `RpcOutcome` envelope shape, and PII-redacted
//! log formatting. Retrieval correctness is the driver's and is covered by
//! the engine's own tests; these deliberately do NOT re-verify it.
//!
//! All five handlers read through the bound driver, and a unit test cannot
//! load the compiled module — so every test that gets past parameter
//! validation binds one. [`bind_without_retrieval`] pins the degrade path;
//! [`bind_recording`] pins what the handler hands the contract and what it
//! does with the answer.
//!
//! One test binds the real in-process driver instead
//! ([`install_memory_driver_for_test`]): the source gate has to be proved end
//! to end, because "the handler passed a scope" and "a restricted profile
//! cannot read another source" are different claims and only the second one
//! is the security property. It is the driver the loadable module wraps,
//! which is as close to production as a test process can get.
//!
//! [`install_memory_driver_for_test`]: crate::openhuman::memory::test_support::install_memory_driver_for_test
use std::sync::{Arc, Mutex};

use super::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::retrieval::{
    FastRetrieveQuery, MemoryRetrieval, RetrievalNodeKind,
};
use crate::openhuman::memory::api::provider::types::{
    ExportPage, ExportRecord, ImportOutcome, SourceScope,
};
use crate::openhuman::memory::api::provider::{
    MemoryCore, MemoryPortability, MemoryProvider, MemoryRecall,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceMemoryHit, NamespaceSummary,
};
use crate::openhuman::memory::source_scope::with_source_scope;
// The engine-backed chunk writes these fixtures need live in
// `retrieval::test_support` rather than here. They are the engine's chunk
// store — `MemoryChunks` is read-only on the contract — and they are
// test-only, which is exactly the pair `direct_engine_refs_tests`'
// line-based scanner cannot tell apart when the reference sits inside an
// inline `#[cfg(test)]` module. See that module's docs.
use tinymemory_api::chunks::{chunk_id, Chunk, Metadata, SourceRef};
use tinymemory_api::null::NullMemoryProvider;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Inert embedder: the driver-backed test below must not reach a real
    // embedding endpoint, and none of these reads rank against a query.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

fn sample_chunk(source: &str, seq: u32) -> Chunk {
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    Chunk {
        id: chunk_id(SourceKind::Chat, source, seq, "test-content"),
        content: format!("content-{source}-{seq}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: source.into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new(format!("slack://{source}/{seq}"))),
            path_scope: None,
        },
        token_count: 20,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    }
}

/// Bind a driver with no retrieval family as `cfg`'s memory driver.
///
/// `FixedDiagnostics` is `NullMemoryProvider`-backed and overrides only
/// `as_maintenance`, so `as_retrieval()` is `None` — the shape of a driver
/// that serves memory without exposing the engine's retrieval primitives.
/// Every test that reaches a handler past its parameter validation needs a
/// binding installed: without one, resolving a driver tries to load the
/// compiled module, which in a test process can block rather than fail.
fn bind_without_retrieval(cfg: &Config) {
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Default::default(),
        Default::default(),
    );
}

/// Bind `driver` as `cfg`'s memory driver and keep a handle on it.
fn bind_recording(cfg: &Config, driver: RecordingRetrieval) -> Arc<RecordingRetrieval> {
    let driver = Arc::new(driver);
    crate::openhuman::memory::binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::clone(&driver) as Arc<dyn MemoryProvider>,
    );
    driver
}

/// One scripted hit, carrying the `tree_kind` every engine-produced hit has.
fn hit(node_id: &str) -> RetrievalHit {
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    RetrievalHit {
        node_id: node_id.to_string(),
        node_kind: RetrievalNodeKind::Leaf,
        tree_id: String::new(),
        tree_kind: Some("source".to_string()),
        tree_scope: String::new(),
        level: 0,
        content: format!("content-{node_id}"),
        entities: Vec::new(),
        topics: Vec::new(),
        time_range_start: ts,
        time_range_end: ts,
        score: 1.0,
        child_ids: Vec::new(),
        source_ref: None,
    }
}

/// What one contract call was handed.
#[derive(Default)]
struct Calls {
    /// The scope argument per member, in call order. Recorded because an
    /// absent scope means UNRESTRICTED on the far side: a handler that
    /// passed `None` where a turn had an allowlist would fail the source
    /// gate open, and the answer would look identical either way.
    scopes: Vec<(String, Option<SourceScope>)>,
    source_queries: Vec<SourceRetrievalQuery>,
    windows: Vec<CoverWindowQuery>,
}

/// A driver whose only advertised behaviour is retrieval: it records the
/// arguments it was handed and answers with scripted hits, or with a
/// rejection when one is scripted.
struct RecordingRetrieval {
    inner: NullMemoryProvider,
    hits: Vec<RetrievalHit>,
    invalid: Option<String>,
    calls: Mutex<Calls>,
}

impl RecordingRetrieval {
    fn new() -> Self {
        Self {
            inner: NullMemoryProvider::new(),
            hits: Vec::new(),
            invalid: None,
            calls: Mutex::new(Calls::default()),
        }
    }

    fn answering(mut self, hits: Vec<RetrievalHit>) -> Self {
        self.hits = hits;
        self
    }

    /// Reject every retrieval call with `message`, the way the engine
    /// rejects an inverted window.
    fn rejecting(mut self, message: &str) -> Self {
        self.invalid = Some(message.to_string());
        self
    }

    fn record(&self, member: &str, scope: Option<&SourceScope>) {
        self.calls
            .lock()
            .expect("calls lock")
            .scopes
            .push((member.to_string(), scope.cloned()));
    }

    /// The scope `member` was handed. The outer `Option` distinguishes
    /// "never called" from "called with no scope".
    fn scope_for(&self, member: &str) -> Option<Option<SourceScope>> {
        self.calls
            .lock()
            .expect("calls lock")
            .scopes
            .iter()
            .find(|(m, _)| m == member)
            .map(|(_, scope)| scope.clone())
    }

    fn source_query(&self) -> SourceRetrievalQuery {
        self.calls
            .lock()
            .expect("calls lock")
            .source_queries
            .first()
            .cloned()
            .expect("retrieve_source was called")
    }

    fn window(&self) -> CoverWindowQuery {
        self.calls
            .lock()
            .expect("calls lock")
            .windows
            .first()
            .cloned()
            .expect("cover_window was called")
    }

    fn answer(&self) -> Result<Vec<RetrievalHit>, MemoryError> {
        match &self.invalid {
            Some(message) => Err(MemoryError::Invalid(message.clone())),
            None => Ok(self.hits.clone()),
        }
    }

    fn page(&self) -> Result<RetrievalResponse, MemoryError> {
        let hits = self.answer()?;
        let total = hits.len();
        Ok(RetrievalResponse {
            hits,
            total,
            truncated: false,
        })
    }
}

#[async_trait]
impl MemoryRetrieval for RecordingRetrieval {
    async fn cover_window(
        &self,
        window: &CoverWindowQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.record("cover_window", scope);
        self.calls
            .lock()
            .expect("calls lock")
            .windows
            .push(window.clone());
        self.page()
    }

    async fn retrieve_source(
        &self,
        query: &SourceRetrievalQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.record("retrieve_source", scope);
        self.calls
            .lock()
            .expect("calls lock")
            .source_queries
            .push(query.clone());
        self.page()
    }

    async fn retrieve_children(
        &self,
        _node_id: &str,
        _max_depth: u32,
        _query: Option<&str>,
        _limit: Option<usize>,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.record("retrieve_children", scope);
        self.answer()
    }

    async fn retrieve_leaves(
        &self,
        _chunk_ids: &[String],
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.record("retrieve_leaves", scope);
        self.answer()
    }

    // The family's remaining members are not reachable from these handlers.
    // They say so rather than returning a plausible empty value that could
    // make a future test pass for the wrong reason.
    async fn fast_retrieve(
        &self,
        _query: &str,
        _options: FastRetrieveQuery,
        _scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        unimplemented!("no retrieval RPC reaches fast_retrieve")
    }

    async fn recall_namespace_scored(
        &self,
        _namespace: &str,
        _query: &str,
        _limit: usize,
        _exclude_session_id: Option<&str>,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        unimplemented!("no retrieval RPC reaches recall_namespace_scored")
    }

    async fn recall_namespace_recent(
        &self,
        _namespace: &str,
        _limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        unimplemented!("no retrieval RPC reaches recall_namespace_recent")
    }

    async fn search_entities(
        &self,
        _query: &str,
        _kinds: Option<&[String]>,
        _limit: usize,
    ) -> Result<Vec<EntityMatch>, MemoryError> {
        unimplemented!("search_entities has its own degrade-path tests")
    }
}

// The mandatory three are supertraits of `MemoryProvider`, so a stub cannot
// skip them. Delegated to the null driver: this double exists to observe
// retrieval, and nothing here stores or recalls.
#[async_trait]
impl MemoryCore for RecordingRetrieval {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        self.inner
            .store(namespace, key, content, category, session_id, taint)
            .await
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.inner.get(namespace, key).await
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.inner.forget(namespace, key).await
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.list(namespace, category, session_id).await
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.inner.namespaces().await
    }
}

#[async_trait]
impl MemoryRecall for RecordingRetrieval {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.recall(query, limit, opts, scope).await
    }
}

#[async_trait]
impl MemoryPortability for RecordingRetrieval {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.inner.export_page(cursor, limit).await
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.inner.import_records(records).await
    }
}

#[async_trait]
impl MemoryProvider for RecordingRetrieval {
    fn driver_id(&self) -> &str {
        "recording-retrieval"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }

    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        Some(self)
    }
}

#[path = "rpc_tests_part_01_tests.rs"]
mod part_01_tests;
