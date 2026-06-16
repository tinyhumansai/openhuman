//! HTTP request and curl download config types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HttpRequestConfig {
    /// Hosts the assistant may open/read via `web_fetch` / `curl`. An exact
    /// host also matches its subdomains; `"*"` allows all public sites; an
    /// empty list blocks all web access. Defaults to `["*"]` so web research
    /// works out of the box — the SSRF guard still blocks local/private hosts
    /// regardless. Narrow this via Settings → Search → Allowed websites.
    #[serde(default = "default_http_allowed_domains")]
    pub allowed_domains: Vec<String>,
    #[serde(default = "default_http_max_response_size")]
    pub max_response_size: usize,
    #[serde(default = "default_http_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for HttpRequestConfig {
    fn default() -> Self {
        Self {
            allowed_domains: default_http_allowed_domains(),
            max_response_size: default_http_max_response_size(),
            timeout_secs: default_http_timeout_secs(),
        }
    }
}

fn default_http_allowed_domains() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_http_max_response_size() -> usize {
    1_000_000
}

fn default_http_timeout_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CurlConfig {
    /// Subdirectory under `workspace_dir` where downloads land. Inputs
    /// are resolved relative to this root; absolute paths and `..`
    /// segments are rejected.
    #[serde(default = "default_curl_dest_subdir")]
    pub dest_subdir: String,
    /// Hard byte ceiling per download. Streaming aborts and the
    /// partial file is removed if exceeded.
    #[serde(default = "default_curl_max_download_bytes")]
    pub max_download_bytes: u64,
    /// Per-request timeout in seconds.
    #[serde(default = "default_curl_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_curl_dest_subdir() -> String {
    "downloads".into()
}

fn default_curl_max_download_bytes() -> u64 {
    50 * 1024 * 1024
}

fn default_curl_timeout_secs() -> u64 {
    120
}

impl Default for CurlConfig {
    fn default() -> Self {
        Self {
            dest_subdir: default_curl_dest_subdir(),
            max_download_bytes: default_curl_max_download_bytes(),
            timeout_secs: default_curl_timeout_secs(),
        }
    }
}
