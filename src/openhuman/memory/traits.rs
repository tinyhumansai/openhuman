//! Core traits and data structures for the OpenHuman memory system.
//!
//! This module defines the foundational `Memory` trait that all storage backends
//! must implement. The standard memory value types (`MemoryEntry`,
//! `MemoryCategory`, `MemoryTaint`, `RecallOpts`, `NamespaceSummary`) are
//! **re-exported from the `tinycortex` crate** (migration W2, spec §0.5): the
//! crate is the single source of truth for these wire-compatible types, and the
//! 30+ host consumers keep their `memory::traits::…` import paths unchanged.
//!
//! `MemoryTaint` is security-critical provenance — it fails closed to
//! `ExternalSync` for unknown/corrupt values so the subconscious gate refuses
//! external-effect tools on chunks of unknown origin. Its semantics were proven
//! byte-identical to the former host definition before re-exporting; the tests
//! below are the host-side seam that pins that contract on the crate type.
//!
//! The `Memory` trait itself stays host-defined for now because of the
//! `sqlite_conn()` escape hatch, which the crate deliberately omits. That hatch
//! is retired in W3 (callers move to `tinycortex::memory::chunks::with_connection`),
//! after which the trait can also become a crate re-export.

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

// ── Value types: re-exported from the crate (W2 type-unification, spec §0.5) ──
//
// These were formerly defined here. They are now the crate's types verbatim
// (identical fields, derives, serde attrs, and — for `MemoryTaint` — the same
// fail-closed `from_db_str`). Re-exporting keeps one source of truth while every
// `use crate::openhuman::memory::traits::{MemoryEntry, …}` site compiles unchanged.
pub use tinycortex::memory::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary, RecallOpts,
};

/// The core trait for memory storage and retrieval.
///
/// Any persistence backend (SQLite, Postgres, Vector DB, etc.) should implement
/// this trait to be used within the OpenHuman ecosystem.
///
/// This mirrors [`tinycortex::memory::Memory`] method-for-method **plus** the
/// host-only [`Memory::sqlite_conn`] escape hatch. It stays host-defined until
/// W3 migrates that hatch to the crate's scoped `with_connection` accessor.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Returns the name of the memory backend (e.g., "sqlite", "vector").
    fn name(&self) -> &str;

    /// Stores a new memory entry or updates an existing one.
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Store an entry with explicit provenance taint.
    ///
    /// Sync paths that ingest text from third-party services (Gmail / Slack /
    /// Notion / Composio / etc.) MUST go through this entry point with
    /// [`MemoryTaint::ExternalSync`] so the subconscious gate can refuse
    /// external_effect tools when the resulting chunks reach a tick's
    /// context window.
    ///
    /// The default implementation degrades to [`Self::store`] for backends
    /// that do not yet persist taint (e.g. mock / in-memory stores used in
    /// tests); the `UnifiedMemory` backend overrides this with a real
    /// taint-carrying upsert.
    async fn store_with_taint(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> anyhow::Result<()> {
        let _ = taint;
        self.store(namespace, key, content, category, session_id)
            .await
    }

    /// Recalls memories matching a query string using keyword or semantic search.
    ///
    /// Namespace is passed via `opts.namespace`; `None` uses the backend's
    /// legacy default namespace (`GLOBAL_NAMESPACE`).
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Recall documents in `namespace` semantically relevant to `query`, keeping
    /// only those whose *vector* similarity to the query is at least
    /// `min_vector_similarity`. Returns `(key, content)` pairs, most-relevant
    /// first — the key lets callers act on the matched entry (e.g. overwrite a
    /// contradicting preference by its topic).
    ///
    /// Unlike [`Self::recall`] (which ranks on a combined keyword + vector +
    /// freshness score), this gates on the vector component alone, so an
    /// unrelated query surfaces nothing — the behaviour Lane-B situational
    /// preferences need. Default returns empty so keyword-only and mock backends
    /// opt out; the unified store overrides it.
    async fn recall_relevant_by_vector(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        min_vector_similarity: f64,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let _ = (namespace, query, limit, min_vector_similarity);
        Ok(Vec::new())
    }

    /// Retrieves a specific memory entry by exact (namespace, key).
    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>>;

    /// Lists memory entries, optionally scoped by namespace, category, session.
    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Deletes a memory entry associated with the given (namespace, key).
    ///
    /// Returns `Ok(true)` if the entry was found and deleted, `Ok(false)` if not found.
    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool>;

    /// Lists all namespaces with aggregate stats, for agent-side discovery.
    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>>;

    /// Returns the total count of all memory entries in the backend.
    async fn count(&self) -> anyhow::Result<usize>;

    /// Performs a health check on the underlying storage system.
    async fn health_check(&self) -> bool;

    /// Return the shared SQLite connection when the backend is `UnifiedMemory`.
    ///
    /// Used by subsystems (e.g. `ArchivistHook`) that need direct SQLite
    /// access for FTS5 / segment writes without going through the async
    /// `Memory` trait.
    ///
    /// Default: `None`. Only `UnifiedMemory` overrides this. Host-only escape
    /// hatch (the crate omits it by design); retired to
    /// `tinycortex::memory::chunks::with_connection` in W3.
    fn sqlite_conn(&self) -> Option<Arc<Mutex<Connection>>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_category_display_outputs_expected_values() {
        assert_eq!(MemoryCategory::Core.to_string(), "core");
        assert_eq!(MemoryCategory::Daily.to_string(), "daily");
        assert_eq!(MemoryCategory::Conversation.to_string(), "conversation");
        assert_eq!(
            MemoryCategory::Custom("project_notes".into()).to_string(),
            "project_notes"
        );
    }

    #[test]
    fn memory_category_serde_uses_snake_case() {
        let core = serde_json::to_string(&MemoryCategory::Core).unwrap();
        let daily = serde_json::to_string(&MemoryCategory::Daily).unwrap();
        let conversation = serde_json::to_string(&MemoryCategory::Conversation).unwrap();

        assert_eq!(core, "\"core\"");
        assert_eq!(daily, "\"daily\"");
        assert_eq!(conversation, "\"conversation\"");
    }

    #[test]
    fn memory_entry_roundtrip_preserves_optional_fields() {
        let entry = MemoryEntry {
            id: "id-1".into(),
            key: "favorite_language".into(),
            content: "Rust".into(),
            namespace: Some("global".into()),
            category: MemoryCategory::Core,
            timestamp: "2026-02-16T00:00:00Z".into(),
            session_id: Some("session-abc".into()),
            score: Some(0.98),
            taint: MemoryTaint::Internal,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MemoryEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "id-1");
        assert_eq!(parsed.key, "favorite_language");
        assert_eq!(parsed.content, "Rust");
        assert_eq!(parsed.namespace.as_deref(), Some("global"));
        assert_eq!(parsed.category, MemoryCategory::Core);
        assert_eq!(parsed.session_id.as_deref(), Some("session-abc"));
        assert_eq!(parsed.score, Some(0.98));
        assert_eq!(parsed.taint, MemoryTaint::Internal);
    }

    #[test]
    fn memory_taint_defaults_to_internal_for_legacy_rows() {
        // Legacy rows persisted before the taint column existed deserialize
        // to MemoryTaint::Internal, so the gate's tainted-subconscious
        // escalation never fires for entries we cannot classify.
        let legacy = r#"{
            "id":"x",
            "key":"k",
            "content":"c",
            "namespace":null,
            "category":"core",
            "timestamp":"2026-01-01T00:00:00Z",
            "session_id":null,
            "score":null
        }"#;
        let parsed: MemoryEntry = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.taint, MemoryTaint::Internal);
    }

    #[test]
    fn memory_taint_as_db_str_uses_snake_case_form() {
        assert_eq!(MemoryTaint::Internal.as_db_str(), "internal");
        assert_eq!(MemoryTaint::ExternalSync.as_db_str(), "external_sync");
    }

    #[test]
    fn memory_taint_from_db_str_known_values_roundtrip_unknown_fails_closed() {
        // Round-trip both known values.
        assert_eq!(
            MemoryTaint::from_db_str(MemoryTaint::Internal.as_db_str()),
            MemoryTaint::Internal
        );
        assert_eq!(
            MemoryTaint::from_db_str(MemoryTaint::ExternalSync.as_db_str()),
            MemoryTaint::ExternalSync
        );
        // Unknown / corrupted column values fail closed to the more
        // restrictive `ExternalSync` so the subconscious gate refuses
        // external_effect tools on chunks of unknown provenance rather
        // than silently treating them as user-authored. This is the W2
        // security seam test on the re-exported crate type.
        assert_eq!(MemoryTaint::from_db_str(""), MemoryTaint::ExternalSync);
        assert_eq!(
            MemoryTaint::from_db_str("EXTERNAL_SYNC"),
            MemoryTaint::ExternalSync
        );
        assert_eq!(
            MemoryTaint::from_db_str("future"),
            MemoryTaint::ExternalSync
        );
    }

    #[test]
    fn memory_taint_roundtrips_external_sync() {
        let entry = MemoryEntry {
            id: "x".into(),
            key: "k".into(),
            content: "c".into(),
            namespace: None,
            category: MemoryCategory::Conversation,
            timestamp: "2026-01-01T00:00:00Z".into(),
            session_id: None,
            score: None,
            taint: MemoryTaint::ExternalSync,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"taint\":\"external_sync\""));
        let parsed: MemoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.taint, MemoryTaint::ExternalSync);
    }
}
