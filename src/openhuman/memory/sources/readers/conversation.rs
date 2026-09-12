//! Product `Config` adapter for the engine-neutral `conversation` reader.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::readers::SourceReader;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

/// Reads `conversation` sources by delegating to [`tinymemory_sources::readers::conversation`].
pub struct ConversationReader;

#[async_trait]
impl SourceReader for ConversationReader {
    fn kind(&self) -> SourceKind {
        SourceKind::Conversation
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        tinymemory_sources::readers::SourceReader::list_items(
            &tinymemory_sources::readers::conversation::ConversationReader,
            source,
            &config.workspace_dir,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        config: &Config,
    ) -> Result<SourceContent, String> {
        tinymemory_sources::readers::SourceReader::read_item(
            &tinymemory_sources::readers::conversation::ConversationReader,
            source,
            item_id,
            &config.workspace_dir,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
