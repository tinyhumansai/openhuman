mod brave;
mod exa;
mod parallel;
mod querit;
mod searxng;
mod seltz;
mod tinyfish;
mod web_search;

pub use brave::{
    BraveImageSearchTool, BraveNewsSearchTool, BraveVideoSearchTool, BraveWebSearchTool,
};
pub use exa::{
    ExaFindSimilarTool, ExaGetContentsTool, ExaResultItem, ExaSearchResponse, ExaSearchTool,
};
pub use parallel::{
    ParallelChatTool, ParallelDatasetTool, ParallelEnrichTool, ParallelExtractTool,
    ParallelResearchTool, ParallelSearchTool, SearchResponse, SearchResultItem,
};
pub use querit::QueritSearchTool;
pub use searxng::{
    normalize_categories, SearxngSearchArgs, SearxngSearchResponse, SearxngSearchTool,
    MAX_RESULTS as SEARXNG_MAX_RESULTS,
};
pub use seltz::SeltzSearchTool;
pub use tinyfish::{TinyFishAgentRunTool, TinyFishFetchTool, TinyFishSearchTool};
pub use web_search::WebSearchTool;
// Crate-internal: the `tools.web_search` RPC reuses the same provider
// resolution so both managed-search surfaces attribute a call identically.
pub(crate) use web_search::resolve_managed_provider;
