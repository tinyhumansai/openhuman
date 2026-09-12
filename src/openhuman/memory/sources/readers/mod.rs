//! Source readers: the [`SourceReader`] trait plus one implementation per
//! [`SourceKind`].
//!
//! A reader knows how to *list* the items available in a source and *read* the
//! content of one item, so ingestion can be driven uniformly across kinds.
//!
//! # Why this is here and not reached through the engine (#5560)
//!
//! It was `tinymemory_core::sources::readers`, re-exported by a glob. Five of
//! the seven readers were already adapters over
//! [`tinymemory_sources::readers`], which this crate now depends on directly;
//! the other two — `composio` and `twitter` — were implemented in the engine
//! crate but named nothing engine-shaped, so they came home as they were.
//!
//! # The trait takes a `&Config`, the crate's takes a `&Path`
//!
//! [`tinymemory_sources::readers::SourceReader`] is deliberately narrower: it
//! takes `workspace: &std::path::Path` and knows nothing about OpenHuman's
//! configuration. This trait is the product-shaped one every host call site
//! already names, so the adapters below are where a `&Config` becomes the
//! workspace path the crate wants. That is the whole of the difference.
//!
//! # `reader_for` hands out network readers, and that is a real decision
//!
//! [`tinymemory_sources::readers::reader_for`] returns `None` for the network
//! kinds **on purpose**. Its docs are explicit: a network reader is meant to be
//! "constructed explicitly by a caller that has already decided the fetch is
//! allowed; it is never handed out by the kind-dispatch that the workspace sync
//! loop drives on a timer", which is what keeps the host in charge of egress,
//! OAuth and cost budgeting.
//!
//! This [`reader_for`] hands out all seven, matching the engine's, because the
//! callers here are RPC handlers acting on an explicit user request — a
//! `memory_sources_*` call naming one source id — and not a timer. **Do not
//! reuse it from a polling loop.** If one ever needs a reader, it should reach
//! for the local kinds through the crate's dispatch and construct a network
//! reader deliberately, so the decision stays visible.

pub mod composio;
pub mod conversation;
pub mod folder;
pub mod github;
pub mod rss;
pub mod twitter;
pub mod web_page;

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

/// A reader that can list items and read content from a memory source.
#[async_trait]
pub trait SourceReader: Send + Sync {
    /// The [`SourceKind`] this reader serves.
    fn kind(&self) -> SourceKind;

    /// List the items currently available in `source`.
    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String>;

    /// Read the content of a single item by its reader-scoped `item_id`.
    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        config: &Config,
    ) -> Result<SourceContent, String>;
}

/// Get the reader for a given source kind.
///
/// Read the module docs before calling this from anything that is not
/// servicing an explicit user request: it hands out the network readers.
#[must_use]
pub fn reader_for(kind: &SourceKind) -> Box<dyn SourceReader> {
    match kind {
        SourceKind::Composio => Box::new(composio::ComposioReader),
        SourceKind::Conversation => Box::new(conversation::ConversationReader),
        SourceKind::Folder => Box::new(folder::FolderReader),
        SourceKind::GithubRepo => Box::new(github::GithubReader),
        SourceKind::TwitterQuery => Box::new(twitter::TwitterReader),
        SourceKind::RssFeed => Box::new(rss::RssReader::new()),
        SourceKind::WebPage => Box::new(web_page::WebPageReader),
    }
}
