//! A driver for the tests of this module's RPC handlers and CLI.
//!
//! # Why a driver has to exist here at all (#5560)
//!
//! The five `tree_summarizer_*` handlers used to call
//! `tree_runtime::{engine, store}` and run the markdown time tree in this
//! process, so a test could write nodes and call a handler with nothing in
//! between. They go through the contract's six runtime-tree doors now, which
//! means they resolve a `MemoryProvider` first — and in a unit test with no
//! binding installed, `memory::binding` tries to load the compiled TinyMemory
//! module, which in a test process can *block* rather than fail.
//! [`bind_tree_driver`] is what stops that, and [`EngineBackedTree`] is what it
//! installs.
//!
//! # Why it is backed by the real engine store rather than a fake
//!
//! What these tests assert is end to end: that an ingest leaves a buffer file
//! on disk carrying the content and its metadata, that a written node comes
//! back with its children, that six nodes read as depth five. Against a
//! hand-rolled fake every one of those becomes an assertion about the fake.
//!
//! The calls below are the ones the real `tinycortex` driver makes for each
//! door — same validators, same order, same error classes — so this double
//! differs from production in exactly one way: *where* the engine runs. Here it
//! is this process; in production it is the loaded module's.
//!
//! Naming `tinymemory_core::` from a `test_support/` path is deliberate and is
//! the route the three sibling globs took when they were deleted:
//! `memory::direct_engine_refs` skips these paths, and the crate is served to
//! them by the `[dev-dependencies]` entry.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::capabilities::{Capabilities, Capability};
use crate::openhuman::memory::api::chunks::Chunk;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::types::{
    ExportPage, ExportRecord, ImportOutcome, SourceScope,
};
use crate::openhuman::memory::api::provider::{
    MemoryCore, MemoryPortability, MemoryProvider, MemoryRecall, MemoryTree,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::tree::{IngestRequest, QueryResult, TreeNode, TreeStatus};
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};

/// The engine's runtime-tree store, for assertions that have to look at what
/// was actually written rather than at what a double said it wrote.
pub(crate) use tinymemory_core::tree::tree_runtime::store as engine_store;

/// A driver serving the six runtime-tree doors from the in-process engine.
pub(crate) struct EngineBackedTree {
    inner: tinymemory_api::null::NullMemoryProvider,
    config: Config,
}

impl EngineBackedTree {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            inner: tinymemory_api::null::NullMemoryProvider::new(),
            config,
        }
    }

    /// An engine failure in the contract's error type.
    ///
    /// Validation refusals go out as [`MemoryError::Invalid`] at the call sites
    /// below rather than through here, matching the driver — that is the
    /// variant `ops::driver_error` unwraps to reproduce the handlers'
    /// historical error strings, so getting the class wrong would show up as a
    /// changed message rather than a failed call.
    fn engine_error(context: &str, error: impl std::fmt::Display) -> MemoryError {
        MemoryError::Other(anyhow::anyhow!("{context}: {error}"))
    }
}

#[async_trait]
impl MemoryCore for EngineBackedTree {
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
impl MemoryRecall for EngineBackedTree {
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
impl MemoryPortability for EngineBackedTree {
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
impl MemoryTree for EngineBackedTree {
    // The family's five **required** members. Nothing under test reaches them,
    // and answering `Unsupported` rather than delegating to null is the honest
    // shape: this double serves the runtime-tree doors and nothing else, so a
    // future test that wandered onto `seal` would get a refusal it can read
    // instead of a silent empty answer it would have to debug.
    async fn append(&self, _request: IngestRequest) -> Result<(), MemoryError> {
        Err(MemoryError::unsupported(Capability::Tree))
    }
    async fn query_source(
        &self,
        _namespace: &str,
        _source_id: &str,
        _limit: usize,
        _scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        Err(MemoryError::unsupported(Capability::Tree))
    }
    async fn drill_down(
        &self,
        _namespace: &str,
        _node_id: &str,
    ) -> Result<QueryResult, MemoryError> {
        Err(MemoryError::unsupported(Capability::Tree))
    }
    async fn seal(&self, _namespace: &str) -> Result<TreeStatus, MemoryError> {
        Err(MemoryError::unsupported(Capability::Tree))
    }
    async fn cascade(&self, _namespace: &str) -> Result<TreeStatus, MemoryError> {
        Err(MemoryError::unsupported(Capability::Tree))
    }

    async fn runtime_buffer_write(
        &self,
        namespace: &str,
        content: &str,
        timestamp: DateTime<Utc>,
        metadata: Option<Value>,
    ) -> Result<String, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        if content.trim().is_empty() {
            return Err(MemoryError::Invalid(
                "content must not be empty".to_string(),
            ));
        }
        let path = engine_store::buffer_write(
            &self.config,
            namespace.trim(),
            content,
            &timestamp,
            metadata.as_ref(),
        )
        .map_err(|error| Self::engine_error("buffer tree content", error))?;
        Ok(path.display().to_string())
    }

    async fn runtime_read_node(
        &self,
        namespace: &str,
        node_id: &str,
    ) -> Result<Option<TreeNode>, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        engine_store::validate_node_id(node_id).map_err(MemoryError::Invalid)?;
        engine_store::read_node(&self.config, namespace.trim(), node_id)
            .map_err(|error| Self::engine_error("read tree node", error))
    }

    async fn runtime_read_children(
        &self,
        namespace: &str,
        parent_id: &str,
    ) -> Result<Vec<TreeNode>, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        engine_store::validate_node_id(parent_id).map_err(MemoryError::Invalid)?;
        engine_store::read_children(&self.config, namespace.trim(), parent_id)
            .map_err(|error| Self::engine_error("read tree children", error))
    }

    async fn runtime_tree_status(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        engine_store::get_tree_status(&self.config, namespace.trim())
            .map_err(|error| Self::engine_error("read tree status", error))
    }

    /// The summariser is resolved through `ops::create_provider` — the resolver
    /// the handler used before the migration — so the tests written around the
    /// local-AI / cloud-opt-in ladder keep asserting against it.
    ///
    /// The real driver resolves through `chat_host::create_chat_model_with_model_id`
    /// instead, which is the one behavioural difference between this double and
    /// production and is documented at `ops::create_provider`.
    async fn runtime_summarize(
        &self,
        namespace: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<TreeNode>, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        let (provider, _model) = super::ops::create_provider(&self.config)
            .map_err(|error| Self::engine_error("create summarizer", error))?;
        tinymemory_core::tree::tree_runtime::engine::run_summarization(
            &self.config,
            provider.as_ref(),
            namespace.trim(),
            timestamp,
        )
        .await
        .map_err(|error| Self::engine_error("run tree summarization", format!("{error:#}")))
    }

    async fn runtime_rebuild(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        engine_store::validate_namespace(namespace).map_err(MemoryError::Invalid)?;
        let (provider, _model) = super::ops::create_provider(&self.config)
            .map_err(|error| Self::engine_error("create summarizer", error))?;
        tinymemory_core::tree::tree_runtime::engine::rebuild_tree(
            &self.config,
            provider.as_ref(),
            namespace.trim(),
        )
        .await
        .map_err(|error| Self::engine_error("rebuild tree", format!("{error:#}")))
    }
}

#[async_trait]
impl MemoryProvider for EngineBackedTree {
    fn driver_id(&self) -> &str {
        "engine-backed-tree"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }
    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }
    fn as_tree(&self) -> Option<&dyn MemoryTree> {
        Some(self)
    }
}

/// Bind [`EngineBackedTree`] as `cfg`'s workspace driver.
///
/// Call this **after** a test has finished shaping its `Config`: the double
/// captures the config it was built with, and several tests flip
/// `local_ai.runtime_enabled` or the cloud opt-in after constructing one.
/// Binding before that would hand the test a driver resolving its summariser
/// from the config it was about to change.
///
/// The binding cache is keyed by workspace + subtree + `[subsystems.memory]`,
/// so a handler's own `binding::for_config` finds exactly what this installed
/// as long as the `[subsystems.memory]` block is untouched.
pub(crate) fn bind_tree_driver(cfg: &Config) {
    crate::openhuman::memory::binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(EngineBackedTree::new(cfg.clone())) as Arc<dyn MemoryProvider>,
    );
}
