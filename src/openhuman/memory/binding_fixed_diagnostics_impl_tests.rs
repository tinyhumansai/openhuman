use super::FixedDiagnostics;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::types::{
    ExportPage, ExportRecord, ImportOutcome, MaintenanceReport, QueueFailure, QueueStats,
    SourceScope, StoreStats,
};
use crate::openhuman::memory::api::provider::{
    MemoryCore, MemoryMaintenance, MemoryPortability, MemoryProvider, MemoryRecall,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};
use async_trait::async_trait;
use tinymemory_api::null::NullMemoryProvider;

impl FixedDiagnostics {
    pub(crate) fn new(store: StoreStats, queue: QueueStats) -> Self {
        Self {
            inner: NullMemoryProvider::new(),
            store,
            queue,
            failure: None,
            backfill: false,
            backfill_trees: Default::default(),
            flush: Default::default(),
            reset: Default::default(),
            retry_calls: std::sync::atomic::AtomicUsize::new(0),
            retry_requeues: 0,
            reembed_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Report `requeued` jobs from [`MemoryMaintenance::retry_failed`].
    pub(crate) fn requeueing(mut self, requeued: u64) -> Self {
        self.retry_requeues = requeued;
        self
    }

    /// Report a backfill running in this driver's process.
    pub(crate) fn backfilling(mut self) -> Self {
        self.backfill = true;
        self
    }

    /// Answer [`MemoryMaintenance::backfill_connector_trees`] with `outcome`.
    ///
    /// Named apart from [`Self::backfilling`], which sets the unrelated
    /// `backfill_in_progress` flag.
    pub(crate) fn backfilling_trees(
        mut self,
        outcome: crate::openhuman::memory::api::provider::types::BackfillTreesOutcome,
    ) -> Self {
        self.backfill_trees = outcome;
        self
    }

    /// Answer [`MemoryMaintenance::flush_pending`] with `outcome`.
    pub(crate) fn flushing(
        mut self,
        outcome: crate::openhuman::memory::api::provider::types::FlushOutcome,
    ) -> Self {
        self.flush = outcome;
        self
    }

    /// Answer [`MemoryMaintenance::reset_derived_index`] with `outcome`.
    pub(crate) fn resetting(
        mut self,
        outcome: crate::openhuman::memory::api::provider::types::ResetOutcome,
    ) -> Self {
        self.reset = outcome;
        self
    }

    /// How many times [`MemoryMaintenance::retry_failed`] has been called.
    pub(crate) fn retry_calls(&self) -> usize {
        self.retry_calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// How many times [`MemoryMaintenance::reembed`] has been called.
    pub(crate) fn reembed_calls(&self) -> usize {
        self.reembed_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl MemoryCore for FixedDiagnostics {
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
impl MemoryRecall for FixedDiagnostics {
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
impl MemoryPortability for FixedDiagnostics {
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
impl MemoryMaintenance for FixedDiagnostics {
    async fn retry_failed(&self) -> Result<MaintenanceReport, MemoryError> {
        self.retry_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(MaintenanceReport {
            operation: "retry_failed".to_string(),
            examined: self.retry_requeues,
            changed: self.retry_requeues,
            findings: Vec::new(),
        })
    }

    async fn reembed(&self) -> Result<MaintenanceReport, MemoryError> {
        self.reembed_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(MaintenanceReport::default())
    }

    async fn compact(&self) -> Result<MaintenanceReport, MemoryError> {
        Ok(MaintenanceReport::default())
    }

    async fn consolidate(&self) -> Result<MaintenanceReport, MemoryError> {
        Ok(MaintenanceReport::default())
    }

    async fn doctor(&self) -> Result<MaintenanceReport, MemoryError> {
        Ok(MaintenanceReport::default())
    }

    async fn store_stats(&self) -> Result<StoreStats, MemoryError> {
        Ok(self.store.clone())
    }

    async fn queue_stats(&self, _kind: Option<&str>) -> Result<QueueStats, MemoryError> {
        Ok(self.queue.clone())
    }

    async fn latest_queue_failure(&self) -> Result<Option<QueueFailure>, MemoryError> {
        Ok(self.failure.clone())
    }

    async fn backfill_in_progress(&self) -> Result<bool, MemoryError> {
        Ok(self.backfill)
    }

    async fn flush_pending(
        &self,
    ) -> Result<crate::openhuman::memory::api::provider::types::FlushOutcome, MemoryError> {
        Ok(self.flush.clone())
    }

    async fn backfill_connector_trees(
        &self,
        _request: crate::openhuman::memory::api::provider::types::BackfillTreesRequest,
    ) -> Result<crate::openhuman::memory::api::provider::types::BackfillTreesOutcome, MemoryError>
    {
        Ok(self.backfill_trees.clone())
    }

    async fn reset_derived_index(
        &self,
    ) -> Result<crate::openhuman::memory::api::provider::types::ResetOutcome, MemoryError> {
        Ok(self.reset.clone())
    }
}

#[async_trait]
impl MemoryProvider for FixedDiagnostics {
    fn driver_id(&self) -> &str {
        "fixed-diagnostics"
    }

    fn capabilities(&self) -> crate::openhuman::memory::api::capabilities::Capabilities {
        crate::openhuman::memory::api::capabilities::Capabilities::all()
    }

    async fn health(&self) -> crate::openhuman::memory::api::health::MemoryHealth {
        crate::openhuman::memory::api::health::MemoryHealth::Ready
    }

    fn as_maintenance(&self) -> Option<&dyn MemoryMaintenance> {
        Some(self)
    }
}
