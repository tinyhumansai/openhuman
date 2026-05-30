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
//! ## Pagination model
//!
//! The official registry uses cursor pagination: each list response carries
//! an opaque `metadata.nextCursor` token (or no token at all when the result
//! set ends). The OpenHuman trait, however, talks in 1-indexed `page`
//! numbers — Smithery's native shape — so this adapter maps page → cursor by
//! caching the cursor that produced each page in a per-process `HashMap`
//! keyed by `(query, page_size, page)`.
//!
//! On a `page > 1` request:
//! - The adapter looks up the cursor that produced `page - 1` in the cache.
//! - **Cache hit**: one HTTP fetch with that cursor.
//! - **Cache miss** (typical after a process restart, or a deep-link to page
//!   N without having walked 1..N-1): the adapter walks `page = 1` forward
//!   sequentially, caching each cursor as it goes, until it has fetched the
//!   requested page. Walks beyond `MAX_CURSOR_WALK_PAGES` bail rather than
//!   risk a DoS — UIs that need deep deep-links should switch to a paging
//!   surface that follows the cursor explicitly.
//!
//! `total_pages` is reported as `page + 1` when the response includes a
//! `nextCursor`, else `page`. Matches the trait doc: "best-effort upper
//! bound — registries that can't compute it report the current page number."
//!
//! ## Response shape
//!
//! The list endpoint wraps each server as `{ "server": { ... }, "_meta": ... }`.
//! The previous DTO assumed a flat shape and silently produced empty
//! summaries when `serde` filled the missing top-level fields with defaults.
//! [`OfficialServerEnvelope`] now matches the real wire shape.
//!
//! Auth: optional `MCP_OFFICIAL_REGISTRY_TOKEN` env var sent as bearer.
//!
//! [repo]: https://github.com/modelcontextprotocol/registry

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::openhuman::config::Config;

use super::super::store;
use super::super::types::{SmitheryConnection, SmitheryServerDetail, SmitheryServerSummary};
use super::{Registry, SOURCE_MCP_OFFICIAL};

const DEFAULT_BASE: &str = "https://registry.modelcontextprotocol.io";

/// Cap on the sequential cursor walk for deep-page cache misses.
///
/// At `page_size = 50` this allows the UI to deep-link up to the 2500th
/// result without a primed cursor cache. Walks past this point bail rather
/// than fan a single user request into hundreds of upstream requests —
/// pagination UIs that need to go deeper should call sequentially so the
/// cache builds up naturally.
const MAX_CURSOR_WALK_PAGES: u32 = 50;

/// Per-process cache mapping `(query, page_size, page)` → cursor that
/// produced *that* page. Cursor for `page = 1` is the empty string (no
/// cursor sent), so we only insert entries for `page >= 2`.
///
/// `parking_lot::Mutex` matches the rest of the memory subsystem and keeps
/// the critical section synchronous — every access is a `HashMap` op, no
/// `.await` while the lock is held.
fn cursor_cache() -> &'static Mutex<HashMap<(String, u32, u32), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u32, u32), String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cursor_cache_get(query: &str, page_size: u32, page: u32) -> Option<String> {
    cursor_cache()
        .lock()
        .get(&(query.to_string(), page_size, page))
        .cloned()
}

fn cursor_cache_set(query: &str, page_size: u32, page: u32, cursor: String) {
    cursor_cache()
        .lock()
        .insert((query.to_string(), page_size, page), cursor);
}

#[cfg(test)]
fn cursor_cache_clear() {
    cursor_cache().lock().clear();
}

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
        let page = page.max(1);

        let cache_key = format!("mcp_official:search:{q}:{page}:{limit}");
        if let Ok(Some(cached_body)) = store::get_cached(config, &cache_key) {
            tracing::debug!(
                "[mcp-official] search cache hit has_query={} q_len={} page={page} limit={limit}",
                !q.is_empty(),
                q.len()
            );
            if let Ok(parsed) = serde_json::from_str::<OfficialListResponse>(&cached_body) {
                let total_pages = total_pages_hint(page, parsed.next_cursor().is_some());
                if let Some(cursor) = parsed.next_cursor() {
                    cursor_cache_set(q, limit, page, cursor.to_string());
                }
                return Ok((parsed.into_summaries(), total_pages));
            }
        }

        let cursor_for_request = if page == 1 {
            None
        } else if let Some(cached) = cursor_cache_get(q, limit, page - 1) {
            // Cache hit: we have the cursor that produced page-1, so one HTTP
            // call gets us page.
            Some(cached)
        } else {
            // Cache miss: walk forward from page 1 until we have a cursor for
            // (page - 1). The walk also primes the cache so subsequent
            // page+1/+2/... requests stay single-hop.
            match walk_cursor_for_page(config, q, limit, page).await? {
                Some(c) => Some(c),
                None => {
                    // The walk ran out of results before reaching `page`.
                    // Return empty + report `page` so the UI stops paging.
                    tracing::debug!(
                        "[mcp-official] walk exhausted has_query={} target_page={page} limit={limit}",
                        !q.is_empty()
                    );
                    return Ok((Vec::new(), page));
                }
            }
        };

        let body = fetch_page(q, limit, cursor_for_request.as_deref()).await?;
        let parsed: OfficialListResponse = serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse MCP official response: {body}"))?;
        let next_cursor = parsed.next_cursor().map(str::to_string);
        let summaries = parsed.into_summaries();

        if let Some(ref c) = next_cursor {
            cursor_cache_set(q, limit, page, c.clone());
        }
        let _ = store::set_cached(config, &cache_key, &body);
        tracing::debug!(
            "[mcp-official] search ok page={page} servers={} has_next={}",
            summaries.len(),
            next_cursor.is_some()
        );
        Ok((summaries, total_pages_hint(page, next_cursor.is_some())))
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

/// Fetch one page from the registry, optionally with a cursor. Returns the
/// raw response body so callers can both parse it and write it to the SQLite
/// response cache.
async fn fetch_page(q: &str, limit: u32, cursor: Option<&str>) -> Result<String> {
    // `q` is user-typed search input — log presence + length only so the
    // diagnostic doesn't leak query text into log aggregators.
    tracing::debug!(
        "[mcp-official] fetch has_query={} q_len={} limit={limit} has_cursor={}",
        !q.is_empty(),
        q.len(),
        cursor.is_some()
    );

    let client = http_client()?;
    let url = format!("{}/v0/servers", base_url());
    let mut req = client.get(&url).header("Accept", "application/json");
    if !q.is_empty() {
        req = req.query(&[("search", q)]);
    }
    req = req.query(&[("limit", &limit.to_string())]);
    if let Some(c) = cursor {
        req = req.query(&[("cursor", c)]);
    }
    req = apply_auth(req);

    let resp = req.send().await.context("MCP official search failed")?;
    let status = resp.status();
    let body = resp.text().await.context("MCP official read failed")?;

    if !status.is_success() {
        tracing::warn!("[mcp-official] search HTTP {status}");
        anyhow::bail!(
            "MCP official registry returned HTTP {status}: {}",
            &body[..body.len().min(200)]
        );
    }
    Ok(body)
}

/// Walk the cursor chain forward starting from page 1 until we have the
/// cursor that, when sent with the next request, produces `target_page`.
///
/// Returns `Some(cursor)` to feed into the request for `target_page`, or
/// `None` if the cursor chain ran out before reaching `target_page`.
///
/// Bails after [`MAX_CURSOR_WALK_PAGES`] iterations to keep a single user
/// request from fanning into hundreds of upstream calls.
async fn walk_cursor_for_page(
    config: &Config,
    q: &str,
    limit: u32,
    target_page: u32,
) -> Result<Option<String>> {
    if target_page <= 1 {
        return Ok(None);
    }
    if target_page > MAX_CURSOR_WALK_PAGES {
        tracing::warn!(
            "[mcp-official] walk refused has_query={} target_page={target_page} max={MAX_CURSOR_WALK_PAGES}",
            !q.is_empty()
        );
        anyhow::bail!(
            "MCP official deep-page walk refused: page={target_page} > MAX_CURSOR_WALK_PAGES={MAX_CURSOR_WALK_PAGES}"
        );
    }

    tracing::debug!(
        "[mcp-official] walk start has_query={} q_len={} target_page={target_page} limit={limit}",
        !q.is_empty(),
        q.len()
    );

    let mut cursor: Option<String> = None;
    let mut net_fetches = 0u32;
    let mut cache_fetches = 0u32;
    // We need the cursor that produces `target_page`, which is the cursor
    // returned by the response for `target_page - 1`.
    for page in 1..target_page {
        let cache_key = format!("mcp_official:search:{q}:{page}:{limit}");

        // Try the persisted SQLite response cache first. After a process
        // restart the in-memory cursor map is empty, but page bodies from a
        // previous run may still be on disk — using them shaves up to N-1
        // HTTP calls off a deep-link walk that has nothing to do with the
        // network's current state.
        let body = match store::get_cached(config, &cache_key) {
            Ok(Some(body)) => {
                cache_fetches += 1;
                body
            }
            _ => {
                let body = fetch_page(q, limit, cursor.as_deref()).await?;
                let _ = store::set_cached(config, &cache_key, &body);
                net_fetches += 1;
                body
            }
        };

        let parsed: OfficialListResponse = serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse MCP official response: {body}"))?;
        let next = parsed.next_cursor().map(str::to_string);

        // Prime the in-memory cursor map as we go so a subsequent direct
        // lookup for `page` doesn't have to re-walk.
        if let Some(ref c) = next {
            cursor_cache_set(q, limit, page, c.clone());
        }

        match next {
            Some(c) => cursor = Some(c),
            None => {
                // Cursor chain exhausted before we reached target_page.
                tracing::debug!(
                    "[mcp-official] walk done (exhausted) page={page} net={net_fetches} cache={cache_fetches}"
                );
                return Ok(None);
            }
        }
    }
    tracing::debug!(
        "[mcp-official] walk done (cursor ready) target_page={target_page} net={net_fetches} cache={cache_fetches}"
    );
    Ok(cursor)
}

/// `total_pages` reporting for the trait contract.
///
/// `has_next` is the boolean derived from `metadata.nextCursor.is_some()` on
/// the current page's response. We can't know the *true* total without
/// walking the entire cursor chain (which is what the bug was originally
/// trying to avoid), so we report `page + 1` when more results exist —
/// matches the trait's "best-effort upper bound" contract and lets the UI
/// render a "next" affordance without overcommitting to a fixed total.
fn total_pages_hint(page: u32, has_next: bool) -> u32 {
    if has_next {
        page.saturating_add(1)
    } else {
        page
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
//
// The real list response wraps each server as
// `{ "server": { ...inner... }, "_meta": { ... } }`. An earlier version of
// this adapter parsed the inner shape at the top level and so silently
// produced empty `OfficialServer` defaults at runtime — the test fixtures
// passed because they were built against the wrong shape too. The envelope
// here matches the actual wire payload (verified against
// `/v0/servers?limit=2` on `registry.modelcontextprotocol.io`).

#[derive(Debug, Clone, Deserialize)]
struct OfficialListResponse {
    #[serde(default)]
    servers: Vec<OfficialServerEnvelope>,
    #[serde(default)]
    metadata: Option<OfficialMetadata>,
}

impl OfficialListResponse {
    fn into_summaries(self) -> Vec<SmitheryServerSummary> {
        self.servers
            .into_iter()
            .map(|env| env.server.into_summary())
            .collect()
    }

    /// Cursor for the *next* page, if the registry indicates there's more.
    /// `None` means the result set ends here.
    fn next_cursor(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.next_cursor.as_deref())
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OfficialMetadata {
    #[serde(default, rename = "nextCursor")]
    next_cursor: Option<String>,
    /// Server-reported count for the *current* page (not the total). Kept
    /// for debug/observability; we don't use it to compute `total_pages`.
    #[serde(default)]
    #[allow(dead_code)]
    count: Option<u32>,
}

/// `{ "server": OfficialServer, "_meta": ... }` envelope.
///
/// `server` is intentionally **not** `#[serde(default)]` — that's exactly
/// the failure mode the wrapper fix is closing out. If upstream ever
/// renames or omits the `server` key, deserialisation must surface as a
/// parse error so the broken wire shape is loud rather than silently
/// producing blank summary cards (the bug this PR was opened to fix).
///
/// `_meta` carries registry-side fields (`status`, `publishedAt`,
/// `isLatest`); we don't need them for summary/detail rendering today, but
/// capturing the whole `Value` keeps the door open without another DTO
/// bump.
#[derive(Debug, Clone, Deserialize)]
struct OfficialServerEnvelope {
    server: OfficialServer,
    #[serde(default, rename = "_meta")]
    #[allow(dead_code)]
    meta: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
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
        assert_eq!(parsed.next_cursor(), None);
    }

    /// The earlier DTO parsed the *inner* shape at the top level, so a real
    /// `{ "server": { ... } }` envelope deserialised into a default-empty
    /// `OfficialServer` and silently produced blank summary cards in the UI.
    /// This regression test pins the wrapper to the real wire shape.
    #[test]
    fn envelope_parses_wrapped_server_payload() {
        let raw = json!({
            "servers": [
                {
                    "server": {
                        "name": "io.github.example/wrapped",
                        "description": "Wrapped server",
                        "remotes": [{ "url": "https://example.com/mcp" }],
                    },
                    "_meta": {
                        "io.modelcontextprotocol.registry/official": {
                            "status": "active",
                            "isLatest": true
                        }
                    }
                }
            ],
            "metadata": { "nextCursor": "tok-xyz", "count": 1 }
        });
        let parsed: OfficialListResponse = serde_json::from_value(raw).unwrap();
        let summaries = parsed.into_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].qualified_name, "io.github.example/wrapped");
        assert_eq!(summaries[0].description.as_deref(), Some("Wrapped server"));
        // `_meta` is preserved as a raw `Value` — no panic on unknown keys.
    }

    /// `metadata.nextCursor` drives both the cursor cache and the
    /// `total_pages` hint. An empty string is treated as "no cursor" so a
    /// future schema bump that stops omitting the field doesn't fool us
    /// into walking forever.
    #[test]
    fn next_cursor_extraction_handles_missing_and_empty() {
        let with_cursor: OfficialListResponse =
            serde_json::from_value(json!({ "servers": [], "metadata": { "nextCursor": "abc" }}))
                .unwrap();
        assert_eq!(with_cursor.next_cursor(), Some("abc"));

        let empty_cursor: OfficialListResponse =
            serde_json::from_value(json!({ "servers": [], "metadata": { "nextCursor": "" }}))
                .unwrap();
        assert_eq!(empty_cursor.next_cursor(), None);

        let no_meta: OfficialListResponse =
            serde_json::from_value(json!({ "servers": [] })).unwrap();
        assert_eq!(no_meta.next_cursor(), None);
    }

    /// Pins the trait-doc contract: report `page + 1` when more pages exist
    /// so the UI renders "next", report `page` when the cursor chain ends
    /// so the UI stops paging. Saturating_add guards against the (silly but
    /// real) `page = u32::MAX` overflow case.
    #[test]
    fn total_pages_hint_reports_best_effort_upper_bound() {
        assert_eq!(total_pages_hint(1, false), 1);
        assert_eq!(total_pages_hint(1, true), 2);
        assert_eq!(total_pages_hint(7, true), 8);
        assert_eq!(total_pages_hint(7, false), 7);
        assert_eq!(total_pages_hint(u32::MAX, true), u32::MAX);
    }

    /// The cursor cache is a process-level singleton keyed by
    /// `(query, page_size, page)`. Confirms reads see what writes wrote,
    /// across queries / page_size partitions, and that the test-only
    /// `cursor_cache_clear` actually drops entries.
    #[test]
    fn cursor_cache_round_trips_and_partitions_by_key() {
        cursor_cache_clear();

        cursor_cache_set("rust", 50, 1, "cur-rust-1".to_string());
        cursor_cache_set("rust", 50, 2, "cur-rust-2".to_string());
        cursor_cache_set("python", 50, 1, "cur-python-1".to_string());
        cursor_cache_set("rust", 25, 1, "cur-rust-25".to_string()); // different page_size

        assert_eq!(
            cursor_cache_get("rust", 50, 1).as_deref(),
            Some("cur-rust-1")
        );
        assert_eq!(
            cursor_cache_get("rust", 50, 2).as_deref(),
            Some("cur-rust-2")
        );
        assert_eq!(
            cursor_cache_get("python", 50, 1).as_deref(),
            Some("cur-python-1")
        );
        assert_eq!(
            cursor_cache_get("rust", 25, 1).as_deref(),
            Some("cur-rust-25")
        );
        // Unrelated key is empty.
        assert_eq!(cursor_cache_get("rust", 50, 99), None);
        assert_eq!(cursor_cache_get("ruby", 50, 1), None);

        cursor_cache_clear();
        assert_eq!(cursor_cache_get("rust", 50, 1), None);
    }

    /// Bare-minimum DoS guard: the deep-page walk refuses to fan one user
    /// request into hundreds of upstream calls.
    #[tokio::test]
    async fn walk_cursor_refuses_above_max_walk_pages() {
        use crate::openhuman::config::Config;
        let config = Config::default();
        let res = walk_cursor_for_page(&config, "anything", 50, MAX_CURSOR_WALK_PAGES + 1).await;
        assert!(res.is_err(), "expected refusal above MAX_CURSOR_WALK_PAGES");
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("MAX_CURSOR_WALK_PAGES"),
            "error should name the limit: {msg}"
        );
    }

    /// `server` is now required on the envelope. A payload that omits or
    /// renames the `server` key must surface as a parse error — the exact
    /// silent-empty-summary failure mode this whole PR was opened to fix.
    /// Without this regression test, dropping `#[serde(default)]` on
    /// `server` could quietly come back in a future "make it more
    /// permissive" change.
    #[test]
    fn envelope_rejects_payload_missing_server_key() {
        // The wrapper has `_meta` but no `server`.
        let raw = json!({
            "servers": [
                { "_meta": { "io.modelcontextprotocol.registry/official": { "status": "active" } } }
            ]
        });
        let parsed = serde_json::from_value::<OfficialListResponse>(raw);
        assert!(
            parsed.is_err(),
            "missing `server` key must be a parse error, not a silent default"
        );

        // And a renamed key ("srv") also fails — defends against an upstream
        // schema rename quietly producing blank cards.
        let renamed = json!({
            "servers": [{ "srv": { "name": "io.github.example/foo" } }]
        });
        assert!(
            serde_json::from_value::<OfficialListResponse>(renamed).is_err(),
            "renamed `server` field must surface as parse error"
        );
    }
}
