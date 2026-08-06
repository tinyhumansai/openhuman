//! Compatibility methods for tinycortex's shared-connection KV store.
//!
//! Every method canonicalizes its namespace/key through
//! [`canonical_identifier`] before delegating. The crate's `set_*` already
//! canonicalizes PII-bearing keys on the way in (#5164) but its `get_*` /
//! `delete_*` / `list_*` address the raw key, so without this shim a write
//! whose key was rewritten reads back as absent — and the caller writes it
//! again. Canonicalizing here is a no-op for the write path (the transform is
//! idempotent and identical) and makes the read path symmetric.

use tinycortex::memory::store::kv::KvStore;

use crate::openhuman::memory::store::namespace_store::UnifiedMemory;
use crate::openhuman::memory::store::safety::canonical_identifier;
use crate::openhuman::memory::store::types::MemoryKvRecord;

impl UnifiedMemory {
    fn tinycortex_kv(&self) -> Result<KvStore, String> {
        KvStore::from_shared_connection(self.conn.clone())
            .map_err(|error| format!("initialize tinycortex KV store: {error}"))
    }

    pub async fn kv_set_global(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        self.tinycortex_kv()?
            .set_global(&canonical_identifier(key), value)
    }

    pub async fn kv_get_global(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        self.tinycortex_kv()?.get_global(&canonical_identifier(key))
    }

    pub async fn kv_set_namespace(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        self.tinycortex_kv()?.set_namespace(
            &canonical_identifier(namespace),
            &canonical_identifier(key),
            value,
        )
    }

    pub async fn kv_get_namespace(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.tinycortex_kv()?
            .get_namespace(&canonical_identifier(namespace), &canonical_identifier(key))
    }

    pub async fn kv_delete_global(&self, key: &str) -> Result<bool, String> {
        self.tinycortex_kv()?
            .delete_global(&canonical_identifier(key))
    }

    pub async fn kv_delete_namespace(&self, namespace: &str, key: &str) -> Result<bool, String> {
        self.tinycortex_kv()?
            .delete_namespace(&canonical_identifier(namespace), &canonical_identifier(key))
    }

    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.tinycortex_kv()?
            .list_namespace(&canonical_identifier(namespace))
    }

    pub(crate) async fn kv_records_for_scope(
        &self,
        namespace: &str,
    ) -> Result<Vec<MemoryKvRecord>, String> {
        self.tinycortex_kv()?
            .records_for_scope(&canonical_identifier(namespace))
            .map(convert_records)
    }

    pub(crate) async fn kv_records_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<MemoryKvRecord>, String> {
        self.tinycortex_kv()?
            .records_namespace(&canonical_identifier(namespace))
            .map(convert_records)
    }

    pub(crate) async fn kv_records_global(&self) -> Result<Vec<MemoryKvRecord>, String> {
        self.tinycortex_kv()?.records_global().map(convert_records)
    }
}

fn convert_records(records: Vec<tinycortex::memory::types::MemoryKvRecord>) -> Vec<MemoryKvRecord> {
    records
        .into_iter()
        .map(|record| MemoryKvRecord {
            namespace: record.namespace,
            key: record.key,
            value: record.value,
            updated_at: record.updated_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::openhuman::inference::embeddings::NoopEmbedding;

    fn test_memory() -> (TempDir, UnifiedMemory) {
        let tmp = TempDir::new().unwrap();
        let memory =
            UnifiedMemory::new(tmp.path(), std::sync::Arc::new(NoopEmbedding), None).unwrap();
        (tmp, memory)
    }

    #[tokio::test]
    async fn global_kv_roundtrips_and_deletes_through_tinycortex() {
        let (_tmp, memory) = test_memory();
        memory.kv_set_global("theme", &json!("dark")).await.unwrap();
        assert_eq!(
            memory.kv_get_global("theme").await.unwrap(),
            Some(json!("dark"))
        );
        assert!(memory.kv_delete_global("theme").await.unwrap());
    }

    #[tokio::test]
    async fn namespace_records_share_the_unified_connection() {
        let (_tmp, memory) = test_memory();
        memory
            .kv_set_namespace("team alpha/#1", "state", &json!({"open": true}))
            .await
            .unwrap();
        let records = memory.kv_records_namespace("team alpha/#1").await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].namespace.as_deref(), Some("team_alpha/_1"));
    }
}
