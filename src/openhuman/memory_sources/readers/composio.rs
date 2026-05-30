//! Composio source reader — delegates to the existing composio sync layer.
//!
//! For Composio sources, `list_items` returns the sync targets and
//! `read_item` is not meaningful (sync is provider-driven, not
//! item-by-item). The reader exists so the registry can uniformly
//! query all source kinds.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

pub struct ComposioReader;

#[async_trait]
impl SourceReader for ComposioReader {
    fn kind(&self) -> SourceKind {
        SourceKind::Composio
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let toolkit = source.toolkit.as_deref().unwrap_or("unknown");
        let connection_id = source.connection_id.as_deref().unwrap_or("unknown");

        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[memory_sources:composio] list_items"
        );

        Ok(vec![SourceItem {
            id: connection_id.to_string(),
            title: format!("{toolkit} connection"),
            updated_at_ms: None,
        }])
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        _config: &Config,
    ) -> Result<SourceContent, String> {
        let toolkit = source.toolkit.as_deref().unwrap_or("unknown");
        Ok(SourceContent {
            id: item_id.to_string(),
            title: format!("{toolkit} sync data"),
            body: format!(
                "Composio {toolkit} data is synced via the provider sync pipeline, not read item-by-item."
            ),
            content_type: ContentType::Plaintext,
            metadata: serde_json::json!({
                "toolkit": toolkit,
                "connection_id": source.connection_id,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_sources::types::MemorySourceEntry;

    fn test_source() -> MemorySourceEntry {
        MemorySourceEntry {
            id: "src_1".into(),
            kind: SourceKind::Composio,
            label: "Gmail".into(),
            enabled: true,
            toolkit: Some("gmail".into()),
            connection_id: Some("cmp_123".into()),
            path: None,
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            selector: None,
        }
    }

    #[tokio::test]
    async fn list_items_returns_connection_as_item() {
        let reader = ComposioReader;
        let config = Config::default();
        let items = reader.list_items(&test_source(), &config).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "cmp_123");
    }
}
