//! The three mandatory families on [`MemoryGuard`] — where steps 3, 4 and 6
//! land for the always-present surface.

use crate::openhuman::memory::api::capabilities::Capability;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::types::{
    ExportPage, ExportRecord, ImportOutcome, SourceScope,
};
use crate::openhuman::memory::api::provider::{MemoryCore, MemoryPortability, MemoryRecall};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};
use async_trait::async_trait;

use super::audit::{trace_allowed, trace_budget, NO_NAMESPACE};
use super::budget::{truncate_content, truncate_entries};
use super::provider::MemoryGuard;

#[async_trait]
impl MemoryCore for MemoryGuard {
    /// Store, with steps 1, 3, 4, 5 and the capture half of 6 applied in that
    /// order: refuse first, then stamp provenance, then redact, then trim.
    ///
    /// Trimming last is deliberate — redaction can lengthen content (a matched
    /// secret becomes `[REDACTED_SECRET]`), so a budget applied before it could
    /// be exceeded by the time the write leaves.
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        let policy = self.policy();
        policy.admit_write(Capability::Core, "core.store", namespace, true)?;

        let taint = policy.stamp_taint(taint);
        let content = policy.redact_outbound(content);
        let content = match policy.capture_budget() {
            Some(max) => truncate_content(&content, max).into_owned(),
            None => content.into_owned(),
        };
        trace_allowed(policy, "core.store", namespace, content.chars().count());

        self.inner()
            .store(namespace, key, &content, category, session_id, taint)
            .await
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let policy = self.policy();
        policy.admit_read(Capability::Core, "core.get", namespace, false)?;
        self.inner().get(namespace, key).await
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        let policy = self.policy();
        policy.admit_write(Capability::Core, "core.forget", namespace, false)?;
        self.inner().forget(namespace, key).await
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let policy = self.policy();
        policy.admit_read(
            Capability::Core,
            "core.list",
            namespace.unwrap_or(NO_NAMESPACE),
            false,
        )?;
        // No recall budget here on purpose: `list` is an enumeration surface
        // (the UI's memory browser, export tooling) rather than the context
        // block a turn injects, and silently truncating it would make the
        // browser disagree with the store. `recall_max_chars` is a *recall*
        // budget and is applied where recall happens.
        self.inner().list(namespace, category, session_id).await
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        let policy = self.policy();
        policy.admit_read(Capability::Core, "core.namespaces", NO_NAMESPACE, false)?;
        self.inner().namespaces().await
    }
}

#[async_trait]
impl MemoryRecall for MemoryGuard {
    /// Recall, with the recall char budget (step 6) applied to the driver's
    /// result.
    ///
    /// ## `scope` is forwarded, never filled from the task-local
    ///
    /// This is the one place the "read the ambient scope at the guard boundary"
    /// rule does **not** apply, and it is deliberate. The embedded driver
    /// *refuses* a `Some(scope)` on recall — see `SCOPE_UNAPPLIED` in
    /// `memory/driver/embedded/recall.rs`, which argues at length that
    /// ignoring the scope is a silent leak and post-filtering is the named
    /// anti-pattern, so refusing is the only honest answer until the recall
    /// predicate exists. Filling `scope` here from
    /// `current_source_scope()` would therefore turn **every** recall issued
    /// inside a `with_source_scope` into a hard error against the only real
    /// driver.
    ///
    /// So the guard passes the caller's `scope` through untouched. The ambient
    /// allowlist is applied where a driver actually implements it as a query
    /// predicate — `MemoryTree::query_source` — and recall joins that list when
    /// the embedded recall path grows the predicate.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let policy = self.policy();
        // The query text itself crosses the boundary on an external driver.
        policy.admit_read(
            Capability::Recall,
            "recall.recall",
            opts.namespace.as_deref().unwrap_or(NO_NAMESPACE),
            true,
        )?;
        let query = policy.redact_outbound(query);

        let hits = self.inner().recall(&query, limit, opts, scope).await?;

        match policy.recall_budget() {
            None => Ok(hits),
            Some(max) => {
                let outcome = truncate_entries(hits, max);
                trace_budget(
                    policy,
                    "recall.recall",
                    outcome.dropped,
                    outcome.trimmed_chars,
                );
                Ok(outcome.entries)
            }
        }
    }
}

#[async_trait]
impl MemoryPortability for MemoryGuard {
    /// Export is **not** budget-trimmed. A truncated export is a corrupt
    /// backup, and portability exists so a binding is reversible — trimming it
    /// would silently make it a one-way door, which is the exact failure the
    /// contract made this family mandatory to avoid.
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        let policy = self.policy();
        policy.admit_read(
            Capability::Portability,
            "portability.export_page",
            NO_NAMESPACE,
            false,
        )?;
        self.inner().export_page(cursor, limit).await
    }

    /// Import **preserves** each record's taint rather than re-stamping it.
    ///
    /// The contract is explicit: "records carry their own taint; an importing
    /// driver must persist what it is given and must not re-stamp provenance."
    /// The guard holds to the same rule. Re-stamping here would rewrite an
    /// entire restored store's provenance to whatever the ambient scope of the
    /// restoring turn happened to be — laundering a million records in one
    /// call, which is the opposite of what step 3 is for.
    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        let policy = self.policy();
        policy.admit_write(
            Capability::Portability,
            "portability.import_records",
            NO_NAMESPACE,
            true,
        )?;
        self.inner().import_records(records).await
    }
}
