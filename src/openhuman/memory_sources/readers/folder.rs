//! Product `Config` adapter for the tinycortex folder reader.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::readers::SourceReader;
use crate::openhuman::memory_sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

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
        tinycortex::memory::sources::SourceReader::list_items(
            &tinycortex::memory::sources::readers::folder::FolderReader,
            source,
            &crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone()),
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
        tinycortex::memory::sources::SourceReader::read_item(
            &tinycortex::memory::sources::readers::folder::FolderReader,
            source,
            item_id,
            &crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone()),
        )
        .await
        .map_err(|error| error.to_string())
    }
}
