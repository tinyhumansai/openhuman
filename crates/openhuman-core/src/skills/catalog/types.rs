//! Domain types for the skill registry. The catalog entry shape and the
//! registry contract are owned by [`tinyskills`].

use serde::Serialize;

use tinyskills::{CatalogEntry, Freshness, RegistryErrorSummary};

/// Entries per page when a caller pages without naming a size.
pub const DEFAULT_PAGE_SIZE: usize = 25;
/// Largest page a caller may ask for.
pub const MAX_PAGE_SIZE: usize = 100;

/// A catalog entry as the RPC and agent tools return it: the [`CatalogEntry`]
/// fields plus the registry it came from and whether it installs directly.
#[derive(Debug, Clone, Serialize)]
pub struct RegistryCatalogEntry {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub registry: String,
    pub installable: bool,
    pub category_label: Option<String>,
}

/// A catalog read: free text plus filters, optionally paged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogQuery {
    pub text: String,
    pub upstreams: Vec<String>,
    pub categories: Vec<String>,
    /// 1-based page. With neither `page` nor `page_size` every match is
    /// returned on one page.
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub force_refresh: bool,
}

impl CatalogQuery {
    pub fn is_paged(&self) -> bool {
        self.page.is_some() || self.page_size.is_some()
    }
}

/// One page of catalog matches and the state of the data behind it.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogPage {
    pub entries: Vec<RegistryCatalogEntry>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub freshness: Freshness,
    /// Unix seconds of the oldest catalog fetch that answered.
    pub fetched_at: Option<u64>,
    /// Whether a source is refreshing in the background now.
    pub refreshing: bool,
    /// The last refresh failure of a source that answered, if any.
    pub last_error: Option<RegistryErrorSummary>,
}

/// Everything the registry knows about one entry.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogDetail {
    #[serde(flatten)]
    pub entry: RegistryCatalogEntry,
    pub overview: String,
    pub install_identifier: Option<String>,
}
