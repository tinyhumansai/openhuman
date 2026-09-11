//! Composio source reader — delegates to the existing composio sync layer.
//!
//! For Composio sources, `list_items` returns the sync targets and
//! `read_item` is not meaningful (sync is provider-driven, not
//! item-by-item). The reader exists so the registry can uniformly
//! query all source kinds.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use crate::openhuman::memory::sources::readers::SourceReader;

/// Lists a Composio connection as a single sync target.
///
/// Composio data arrives through the provider sync pipeline rather than
/// item-by-item, so `read_item` returns a description of that rather than
/// content. The reader exists so the registry can query every source kind
/// uniformly.
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
#[path = "composio_tests.rs"]
mod tests;
