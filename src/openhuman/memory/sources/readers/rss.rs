//! Product `Config` adapter for the engine-neutral `rss` reader.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::readers::SourceReader;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

/// Reads `rss_feed` sources by delegating to
/// [`tinymemory_sources::readers::rss`].
///
/// The inner reader caches a freshly fetched feed between `list_items` and
/// `read_item`, so constructing it per trait call would turn one sync into N+1
/// downloads. That is why this one owns its delegate where the others are
/// unit structs.
pub struct RssReader {
    inner: tinymemory_sources::readers::rss::RssReader,
}

impl RssReader {
    /// Build a reader with an empty feed cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: tinymemory_sources::readers::rss::RssReader::new(),
        }
    }
}

impl Default for RssReader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SourceReader for RssReader {
    fn kind(&self) -> SourceKind {
        SourceKind::RssFeed
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        tinymemory_sources::readers::SourceReader::list_items(
            &self.inner,
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
            &self.inner,
            source,
            item_id,
            &config.workspace_dir,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
