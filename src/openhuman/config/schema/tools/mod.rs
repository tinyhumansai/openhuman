//! Tool-related config: browser, HTTP, web search, composio, secrets, multimodal.

pub mod browser;
pub mod http;
pub mod integrations;
pub mod mcp;
pub mod multimodal;
pub mod search;

pub use browser::{BrowserComputerUseConfig, BrowserConfig};
pub use http::{CurlConfig, HttpRequestConfig};
pub use integrations::{
    ComposioConfig, ComputerControlConfig, IntegrationToggle, IntegrationsConfig,
    PolymarketClobCredentials, PolymarketConfig, SecretsConfig, COMPOSIO_MODE_BACKEND,
    COMPOSIO_MODE_DIRECT, INTEGRATION_MODE_BYO, INTEGRATION_MODE_MANAGED,
};
pub use mcp::{
    GitbooksConfig, HttpHeader, McpAuthConfig, McpClientConfig, McpClientIdentityConfig,
    McpRegistryAuthConfig, McpServerConfig,
};
pub use multimodal::{MultimodalConfig, MultimodalFileConfig};
pub use search::{
    SearchConfig, SearchEngine, SearchEngineCredentials, SearxngConfig, SeltzConfig,
    WebSearchConfig, SEARCH_ENGINE_BRAVE, SEARCH_ENGINE_DISABLED, SEARCH_ENGINE_MANAGED,
    SEARCH_ENGINE_PARALLEL, SEARCH_ENGINE_QUERIT,
};
