//! Smithery.ai MCP registry HTTP client.
//!
//! Base URL: <https://registry.smithery.ai>
//! Public endpoints:
//!   `GET /servers?q=<query>&page=N&pageSize=M` → `SmitheryListResponse`
//!   `GET /servers/{qualifiedName}` → `SmitheryServerDetail`
//!
//! Results are cached in SQLite for 10 minutes (TTL controlled in `store.rs`).
//! Auth: optional `SMITHERY_API_KEY` env var sent as `Authorization: Bearer`.

use anyhow::{Context, Result};
use reqwest::Client;

use crate::openhuman::config::Config;

use super::store;
use super::types::{SmitheryListResponse, SmitheryServerDetail, SmitheryServerSummary};

const SMITHERY_BASE: &str = "https://registry.smithery.ai";
const DEFAULT_PAGE_SIZE: u32 = 20;

fn smithery_client() -> Result<Client> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("Failed to build Smithery HTTP client")
}

fn smithery_api_key() -> Option<String> {
    std::env::var("SMITHERY_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

fn apply_auth(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(key) = smithery_api_key() {
        builder.bearer_auth(key)
    } else {
        builder
    }
}

/// Search the Smithery registry. Results are cached in SQLite.
pub async fn registry_search(
    config: &Config,
    query: Option<&str>,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SmitheryServerSummary>, u32)> {
    let page = page.max(1);
    let page_size = if page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        page_size
    };
    let q = query.unwrap_or("").trim();

    let cache_key = format!("search:{}:{}:{}", q, page, page_size);

    // Check SQLite cache first
    if let Ok(Some(cached_body)) = store::get_cached(config, &cache_key) {
        tracing::debug!("[mcp-client] registry_search cache hit key={}", cache_key);
        if let Ok(resp) = serde_json::from_str::<SmitheryListResponse>(&cached_body) {
            return Ok((resp.servers, resp.pagination.total_pages));
        }
    }

    tracing::debug!(
        "[mcp-client] registry_search fetching q={:?} page={} page_size={}",
        q,
        page,
        page_size
    );

    let client = smithery_client()?;
    let mut req = client.get(format!("{SMITHERY_BASE}/servers"));
    if !q.is_empty() {
        req = req.query(&[("q", q)]);
    }
    req = req
        .query(&[
            ("page", &page.to_string()),
            ("pageSize", &page_size.to_string()),
        ])
        .header("Accept", "application/json");
    req = apply_auth(req);

    let resp = req
        .send()
        .await
        .context("Smithery registry_search request failed")?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .context("Failed to read Smithery response body")?;

    if !status.is_success() {
        tracing::warn!(
            "[mcp-client] registry_search HTTP {} for key={}",
            status,
            cache_key
        );
        anyhow::bail!(
            "Smithery registry returned HTTP {}: {}",
            status,
            &body[..body.len().min(200)]
        );
    }

    let parsed: SmitheryListResponse = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse Smithery list response: {body}"))?;

    let total_pages = parsed.pagination.total_pages;
    let servers = parsed.servers.clone();

    // Cache success
    let _ = store::set_cached(config, &cache_key, &body);
    tracing::debug!(
        "[mcp-client] registry_search ok servers={} total_pages={}",
        servers.len(),
        total_pages
    );

    Ok((servers, total_pages))
}

/// Fetch details for one server. Results are cached in SQLite.
pub async fn registry_get(config: &Config, qualified_name: &str) -> Result<SmitheryServerDetail> {
    let cache_key = format!("detail:{qualified_name}");

    if let Ok(Some(cached_body)) = store::get_cached(config, &cache_key) {
        tracing::debug!(
            "[mcp-client] registry_get cache hit qualified_name={}",
            qualified_name
        );
        if let Ok(detail) = serde_json::from_str::<SmitheryServerDetail>(&cached_body) {
            return Ok(detail);
        }
    }

    tracing::debug!(
        "[mcp-client] registry_get fetching qualified_name={}",
        qualified_name
    );

    let client = smithery_client()?;
    let url = format!(
        "{SMITHERY_BASE}/servers/{}",
        urlencoding_encode(qualified_name)
    );
    let req = apply_auth(client.get(&url).header("Accept", "application/json"));

    let resp = req
        .send()
        .await
        .context("Smithery registry_get request failed")?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .context("Failed to read Smithery detail response")?;

    if !status.is_success() {
        anyhow::bail!(
            "Smithery registry GET {} returned HTTP {}: {}",
            qualified_name,
            status,
            &body[..body.len().min(200)]
        );
    }

    let detail: SmitheryServerDetail = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse Smithery detail: {body}"))?;

    let _ = store::set_cached(config, &cache_key, &body);
    tracing::debug!(
        "[mcp-client] registry_get ok qualified_name={} connections={}",
        qualified_name,
        detail.connections.len()
    );

    Ok(detail)
}

/// Minimal URL percent-encoding for path segments (encodes `/` and common specials).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_encode_handles_at_sign_and_slash() {
        // @ is kept (valid in registry names like @modelcontextprotocol/server-fs)
        // / is encoded so it does not split the URL path
        let encoded = urlencoding_encode("@modelcontextprotocol/server-filesystem");
        assert!(encoded.contains('%'), "slash should be encoded: {encoded}");
        assert!(encoded.contains('@'), "@ should be preserved: {encoded}");
    }

    #[test]
    fn urlencoding_encode_plain_ascii_unchanged() {
        assert_eq!(urlencoding_encode("simple-name"), "simple-name");
    }

    #[test]
    fn urlencoding_encode_space_becomes_percent_20() {
        let encoded = urlencoding_encode("hello world");
        assert_eq!(encoded, "hello%20world");
    }
}
