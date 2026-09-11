//! Product `Config` adapter for the engine-neutral `github` reader.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::readers::SourceReader;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

pub use tinymemory_sources::readers::github::{repo_archive_source_id, repo_chunk_scope};

/// Reads `github_repo` sources by delegating to
/// [`tinymemory_sources::readers::github`].
pub struct GithubReader;

#[async_trait]
impl SourceReader for GithubReader {
    fn kind(&self) -> SourceKind {
        SourceKind::GithubRepo
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        tinymemory_sources::readers::SourceReader::list_items(
            &tinymemory_sources::readers::github::GithubReader,
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
            &tinymemory_sources::readers::github::GithubReader,
            source,
            item_id,
            &config.workspace_dir,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
