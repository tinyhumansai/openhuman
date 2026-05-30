//! Source reader trait and per-kind implementations.

pub mod composio;
pub mod folder;
pub mod github;
pub mod rss;
pub mod twitter;
pub mod web_page;

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

/// A reader that can list items and read content from a memory source.
#[async_trait]
pub trait SourceReader: Send + Sync {
    fn kind(&self) -> SourceKind;
    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String>;
    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        config: &Config,
    ) -> Result<SourceContent, String>;
}

/// Get the reader for a given source kind.
pub fn reader_for(kind: &SourceKind) -> Box<dyn SourceReader> {
    match kind {
        SourceKind::Composio => Box::new(composio::ComposioReader),
        SourceKind::Folder => Box::new(folder::FolderReader),
        SourceKind::GithubRepo => Box::new(github::GithubReader),
        SourceKind::TwitterQuery => Box::new(twitter::TwitterReader),
        SourceKind::RssFeed => Box::new(rss::RssReader),
        SourceKind::WebPage => Box::new(web_page::WebPageReader),
    }
}
