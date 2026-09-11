//! Product `Config` adapter for the engine-neutral `folder` reader.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::readers::SourceReader;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

/// Reads `folder` sources by delegating to [`tinymemory_sources::readers::folder`].
pub struct FolderReader;

#[async_trait]
impl SourceReader for FolderReader {
    fn kind(&self) -> SourceKind {
        SourceKind::Folder
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        tinymemory_sources::readers::SourceReader::list_items(
            &tinymemory_sources::readers::folder::FolderReader,
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
            &tinymemory_sources::readers::folder::FolderReader,
            source,
            item_id,
            &config.workspace_dir,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
