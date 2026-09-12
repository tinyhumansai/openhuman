//! Twitter/X query source reader.
//!
//! Fetches tweets matching a search query. Uses the Twitter API v2
//! search endpoint. Requires bearer token configuration (not yet
//! wired — this reader validates the source config and returns a
//! clear error when no credentials are available).

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use crate::openhuman::memory::sources::readers::SourceReader;

const DEFAULT_SINCE_DAYS: u32 = 7;

/// Reads `twitter_query` sources.
///
/// Unimplemented: the Twitter API v2 search endpoint needs a bearer token and
/// that credential wiring has not landed, so both methods validate the source
/// and return an error naming what is missing.
pub struct TwitterReader;

#[async_trait]
impl SourceReader for TwitterReader {
    fn kind(&self) -> SourceKind {
        SourceKind::TwitterQuery
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let query = source
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or("twitter source requires a non-empty query")?;
        let _since_days = source.since_days.unwrap_or(DEFAULT_SINCE_DAYS);

        tracing::debug!(
            query = %query,
            "[memory_sources:twitter] list_items"
        );

        // Twitter API v2 requires a bearer token. For now, return an
        // informative error until credential wiring lands.
        Err(format!(
            "Twitter API integration not yet configured. Query '{query}' is saved and will \
             sync once a Twitter bearer token is provided in settings."
        ))
    }

    async fn read_item(
        &self,
        _source: &MemorySourceEntry,
        item_id: &str,
        _config: &Config,
    ) -> Result<SourceContent, String> {
        tracing::debug!(
            item_id = %item_id,
            "[memory_sources:twitter] read_item"
        );

        Err("Twitter API integration not yet configured. \
             Individual tweet reading requires a bearer token."
            .to_string())
    }
}

#[cfg(test)]
#[path = "twitter_tests.rs"]
mod tests;
