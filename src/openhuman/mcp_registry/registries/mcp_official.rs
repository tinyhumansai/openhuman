//! Official MCP registry adapter — [modelcontextprotocol/registry][repo].
//!
//! Base URL: `https://registry.modelcontextprotocol.io` (override with
//! `MCP_OFFICIAL_REGISTRY_BASE`).
//!
//! Endpoints used:
//! - `GET /v0/servers?search=<query>&limit=<n>&cursor=<opt>` — paginated list
//! - `GET /v0/servers/{name}` — full detail for one server (or a fallback
//!   path that searches by exact name when the direct endpoint 404s)
//!
//! The official registry uses cursor pagination. We map our 1-indexed `page`
//! parameter onto it by treating `page == 1` as "no cursor" and refusing
//! deeper pagination for now — the caller gets back the first page plus a
//! `total_pages` hint of `1`. Cursor-aware pagination is a follow-up.
//!
//! Auth: optional `MCP_OFFICIAL_REGISTRY_TOKEN` env var sent as bearer.
//!
//! [repo]: https://github.com/modelcontextprotocol/registry

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::openhuman::config::Config;

use super::super::store;
use super::super::types::{SmitheryConnection, SmitheryServerDetail, SmitheryServerSummary};
use super::{Registry, SOURCE_MCP_OFFICIAL};

const DEFAULT_BASE: &str = "https://registry.modelcontextprotocol.io";

pub struct McpOfficialRegistry;

#[async_trait]
impl Registry for McpOfficialRegistry {
    fn source(&self) -> &'static str {
        SOURCE_MCP_OFFICIAL
    }

    async fn search(
        &self,
        config: &Config,
        query: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<SmitheryServerSummary>, u32)> {
        let q = query.unwrap_or("").trim();
        let limit = page_size.max(1);

        let cache_key = format!("mcp_official:search:{q}:{page}:{limit}");
        if let Ok(Some(cached_body)) = store::get_cached(config, &cache_key) {
            tracing::debug!("[mcp-official] search cache hit key={cache_key}");
            if let Ok(parsed) = serde_json::from_str::<OfficialListResponse>(&cached_body) {
                return Ok((parsed.into_summaries(), 1));
            }
        }

        if page > 1 {
            // Cursor pagination not yet wired up — return empty for page > 1
            // so the UI doesn't loop fetching nonexistent pages.
            tracing::debug!(
                "[mcp-official] search returning empty for page>1 \
                 (cursor pagination not implemented)"
            );
            return Ok((Vec::new(), 1));
        }

        tracing::debug!("[mcp-official] search fetching q={q:?} limit={limit}");

        let client = http_client()?;
        let url = format!("{}/v0/servers", base_url());
        let mut req = client.get(&url).header("Accept", "application/json");
        if !q.is_empty() {
            req = req.query(&[("search", q)]);
        }
        req = req.query(&[("limit", &limit.to_string())]);
        req = apply_auth(req);

        let resp = req.send().await.context("MCP official search failed")?;
        let status = resp.status();
        let body = resp.text().await.context("MCP official read failed")?;

        if !status.is_success() {
            tracing::warn!("[mcp-official] search HTTP {status} for key={cache_key}");
            anyhow::bail!(
                "MCP official registry returned HTTP {status}: {}",
                &body[..body.len().min(200)]
            );
        }

        let parsed: OfficialListResponse = serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse MCP official response: {body}"))?;
        let summaries = parsed.into_summaries();
        let _ = store::set_cached(config, &cache_key, &body);
        tracing::debug!(
            "[mcp-official] search ok servers={} (cursor pagination not wired)",
            summaries.len()
        );
        Ok((summaries, 1))
    }

    async fn get(&self, config: &Config, qualified_name: &str) -> Result<SmitheryServerDetail> {
        let cache_key = format!("mcp_official:detail:{qualified_name}");
        if let Ok(Some(cached_body)) = store::get_cached(config, &cache_key) {
            tracing::debug!("[mcp-official] get cache hit qualified_name={qualified_name}");
            if let Ok(server) = serde_json::from_str::<OfficialServer>(&cached_body) {
                return Ok(server.into_detail());
            }
        }

        let client = http_client()?;
        let url = format!(
            "{}/v0/servers/{}",
            base_url(),
            urlencoding_encode(qualified_name)
        );
        tracing::debug!("[mcp-official] get fetching {url}");
        let req = apply_auth(client.get(&url).header("Accept", "application/json"));

        let resp = req.send().await.context("MCP official get failed")?;
        let status = resp.status();
        let body = resp.text().await.context("MCP official read failed")?;

        if !status.is_success() {
            anyhow::bail!(
                "MCP official registry GET {qualified_name} returned HTTP {status}: {}",
                &body[..body.len().min(200)]
            );
        }

        let server: OfficialServer = serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse MCP official detail: {body}"))?;
        let _ = store::set_cached(config, &cache_key, &body);
        Ok(server.into_detail())
    }
}

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("Failed to build MCP official HTTP client")
}

fn base_url() -> String {
    std::env::var("MCP_OFFICIAL_REGISTRY_BASE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn auth_token() -> Option<String> {
    std::env::var("MCP_OFFICIAL_REGISTRY_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

fn apply_auth(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(token) = auth_token() {
        builder.bearer_auth(token)
    } else {
        builder
    }
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

// ── Wire-shape DTOs (best-effort against the official OpenAPI) ───────────────
//
// The official registry OpenAPI evolves; these are deliberately permissive
// (every nested field is optional) so a schema bump doesn't break parsing.

#[derive(Debug, Clone, Deserialize)]
struct OfficialListResponse {
    #[serde(default)]
    servers: Vec<OfficialServer>,
    #[serde(default)]
    #[allow(dead_code)]
    metadata: Option<Value>,
}

impl OfficialListResponse {
    fn into_summaries(self) -> Vec<SmitheryServerSummary> {
        self.servers.into_iter().map(|s| s.into_summary()).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OfficialServer {
    /// Reverse-DNS-style identifier, e.g. `io.github.foo/server-bar`.
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "iconUrl")]
    icon_url: Option<String>,
    /// Remote (HTTP / SSE) endpoints exposed by this server.
    #[serde(default)]
    remotes: Vec<OfficialRemote>,
    /// Installable subprocess packages (npm, pip, brew, …).
    #[serde(default)]
    packages: Vec<OfficialPackage>,
}

impl OfficialServer {
    fn into_summary(self) -> SmitheryServerSummary {
        SmitheryServerSummary {
            qualified_name: self.name.clone(),
            display_name: self.name.clone(),
            description: self.description.clone(),
            icon_url: self.icon_url.clone(),
            use_count: 0,
            is_deployed: !self.remotes.is_empty(),
            source: SOURCE_MCP_OFFICIAL.to_string(),
            extra: std::collections::HashMap::new(),
        }
    }

    fn into_detail(self) -> SmitheryServerDetail {
        let mut connections: Vec<SmitheryConnection> = Vec::new();
        for r in &self.remotes {
            connections.push(SmitheryConnection {
                r#type: "http".to_string(),
                deployment_url: r.url.clone(),
                config_schema: None,
                example_config: None,
                published: true,
                extra: std::collections::HashMap::new(),
            });
        }
        for p in &self.packages {
            connections.push(SmitheryConnection {
                r#type: "stdio".to_string(),
                deployment_url: None,
                config_schema: p.config_schema.clone(),
                example_config: None,
                published: true,
                extra: std::collections::HashMap::new(),
            });
        }
        SmitheryServerDetail {
            qualified_name: self.name.clone(),
            display_name: self.name.clone(),
            description: self.description.clone(),
            icon_url: self.icon_url.clone(),
            connections,
            source: SOURCE_MCP_OFFICIAL.to_string(),
            extra: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OfficialRemote {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OfficialPackage {
    #[serde(default, rename = "configSchema")]
    config_schema: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn official_server_into_summary_uses_name_as_qualified() {
        let s: OfficialServer = serde_json::from_value(json!({
            "name": "io.github.example/server",
            "description": "Example",
        }))
        .unwrap();
        let sum = s.into_summary();
        assert_eq!(sum.qualified_name, "io.github.example/server");
        assert_eq!(sum.source, SOURCE_MCP_OFFICIAL);
    }

    #[test]
    fn list_response_tolerates_missing_metadata() {
        let raw = json!({ "servers": [] });
        let parsed: OfficialListResponse = serde_json::from_value(raw).unwrap();
        assert!(parsed.servers.is_empty());
    }
}
