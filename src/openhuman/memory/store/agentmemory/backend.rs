//! `impl Memory for AgentMemoryBackend` — the hot path for OpenHuman <→
//! agentmemory traffic.
//!
//! The upstream agentmemory REST contract (endpoints, payloads, lifecycle
//! semantics) lives at <https://github.com/rohitg00/agentmemory>. This
//! module pins the OpenHuman-visible projection of that contract; the
//! field-mapping table is in `mapping.rs` and the security guard is in
//! `client.rs`.

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::MemoryConfig;
use crate::openhuman::memory::traits::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};

use super::client::AgentMemoryClient;
use super::mapping::{
    ForgetRequest, ForgetResponse, HealthResponse, MemoriesResponse, ProjectsResponse,
    RememberRequest, RememberResponse, SmartSearchRequest, SmartSearchResponse, WireMemory,
    DEFAULT_PROJECT,
};

/// Memory backend that proxies every trait call through agentmemory's REST
/// surface. Construct via [`AgentMemoryBackend::from_config`].
pub struct AgentMemoryBackend {
    client: AgentMemoryClient,
}

impl AgentMemoryBackend {
    /// Build from a [`MemoryConfig`]. Reads the optional
    /// `agentmemory_url` / `agentmemory_secret` / `agentmemory_timeout_ms`
    /// fields and falls back to documented defaults
    /// (`http://localhost:3111`, no secret, 5000ms timeout).
    pub fn from_config(config: &MemoryConfig) -> Result<Self> {
        let client = AgentMemoryClient::new(
            config.agentmemory_url.as_deref(),
            config.agentmemory_secret.as_deref(),
            config.agentmemory_timeout_ms,
        )?;
        log::debug!(
            "[memory::agentmemory] backend initialised against {}",
            client.base()
        );
        Ok(Self { client })
    }
}

fn namespace_or_default(ns: &str) -> &str {
    if ns.is_empty() {
        DEFAULT_PROJECT
    } else {
        ns
    }
}

/// Lookup cap for `get()` / `forget()` exact-title resolution.
///
/// agentmemory does not expose a `(project, title)` lookup endpoint, so
/// `get()` fans out via smart-search and filters client-side for an exact
/// title match. A small cap (e.g. 5) drops valid exact matches that rank
/// lower in BM25+vector score. 100 is high enough that an exact title
/// never falls off the page in practice while keeping the response
/// payload bounded.
const EXACT_LOOKUP_LIMIT: usize = 100;

#[async_trait]
impl Memory for AgentMemoryBackend {
    fn name(&self) -> &str {
        "agentmemory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        log::debug!(
            "[memory::agentmemory] store namespace={namespace:?} key={key:?} session_id={session_id:?} category={category:?}"
        );
        let body = RememberRequest::build(
            namespace_or_default(namespace),
            key,
            content,
            &category,
            session_id,
        );
        let _: RememberResponse = self.client.post_json("agentmemory/remember", &body).await?;
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        log::debug!(
            "[memory::agentmemory] recall query={query:?} limit={limit} namespace={:?} category={:?} session={:?} min_score={:?}",
            opts.namespace, opts.category, opts.session_id, opts.min_score,
        );
        let project = opts.namespace.map(|s| {
            if s.is_empty() {
                DEFAULT_PROJECT.to_string()
            } else {
                s.to_string()
            }
        });
        let body = SmartSearchRequest {
            query: query.to_string(),
            limit,
            project,
        };
        let resp: SmartSearchResponse = self
            .client
            .post_json("agentmemory/smart-search", &body)
            .await?;

        let mut entries: Vec<MemoryEntry> = resp
            .results
            .into_iter()
            .map(WireMemory::into_entry)
            .collect();
        let before = entries.len();

        if let Some(cat) = opts.category.as_ref() {
            entries.retain(|e| &e.category == cat);
        }
        if let Some(session) = opts.session_id {
            entries.retain(|e| e.session_id.as_deref() == Some(session));
        }
        if let Some(min_score) = opts.min_score {
            // Scoreless rows (e.g. direct fetches that never went through
            // smart-search) cannot prove they meet the threshold — drop
            // them rather than letting them through silently.
            entries.retain(|e| e.score.is_some_and(|s| s >= min_score));
        }
        if entries.len() != before {
            log::trace!(
                "[memory::agentmemory] recall client-filter retained {}/{} hits",
                entries.len(),
                before,
            );
        }
        Ok(entries)
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>> {
        log::debug!("[memory::agentmemory] get namespace={namespace:?} key={key:?}");
        let project = namespace_or_default(namespace);
        let body = SmartSearchRequest {
            query: key.to_string(),
            limit: EXACT_LOOKUP_LIMIT,
            project: Some(project.to_string()),
        };
        let resp: SmartSearchResponse = self
            .client
            .post_json("agentmemory/smart-search", &body)
            .await?;
        let hit = resp
            .results
            .into_iter()
            .find(|r| r.title.as_deref() == Some(key))
            .map(WireMemory::into_entry);
        log::trace!(
            "[memory::agentmemory] get namespace={namespace:?} key={key:?} matched={}",
            hit.is_some()
        );
        Ok(hit)
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        log::debug!(
            "[memory::agentmemory] list namespace={namespace:?} category={category:?} session_id={session_id:?}"
        );
        // When the caller passes Some(""), normalise to the "default"
        // project so the wire query stays consistent. When they pass
        // None, list across every project — matching the trait's
        // optional-namespace contract.
        let path = match namespace {
            Some(ns) => format!(
                "agentmemory/memories?latest=true&project={}",
                url_encode(namespace_or_default(ns))
            ),
            None => "agentmemory/memories?latest=true".to_string(),
        };
        let resp: MemoriesResponse = self.client.get_json(&path).await?;
        let mut entries: Vec<MemoryEntry> = resp
            .memories
            .into_iter()
            .map(WireMemory::into_entry)
            .collect();
        let before = entries.len();
        if let Some(cat) = category {
            entries.retain(|e| &e.category == cat);
        }
        if let Some(session) = session_id {
            entries.retain(|e| e.session_id.as_deref() == Some(session));
        }
        if entries.len() != before {
            log::trace!(
                "[memory::agentmemory] list client-filter retained {}/{} rows",
                entries.len(),
                before,
            );
        }
        Ok(entries)
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool> {
        log::debug!("[memory::agentmemory] forget namespace={namespace:?} key={key:?}");
        // agentmemory's /forget takes an id, not (project, title). Look
        // the key up first via smart-search (mirrors `get` above), then
        // POST /forget against that id. If no exact title match exists,
        // return Ok(false) — same contract as the SQLite backend's
        // delete-by-(namespace, key).
        let Some(target) = self.get(namespace, key).await? else {
            log::trace!(
                "[memory::agentmemory] forget namespace={namespace:?} key={key:?} unresolved -> noop"
            );
            return Ok(false);
        };
        let body = ForgetRequest {
            id: target.id.clone(),
        };
        let resp: ForgetResponse = self.client.post_json("agentmemory/forget", &body).await?;
        log::debug!(
            "[memory::agentmemory] forget namespace={namespace:?} key={key:?} id={} forgotten={}",
            target.id,
            resp.forgotten,
        );
        Ok(resp.forgotten)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        log::debug!("[memory::agentmemory] namespace_summaries");
        let resp: ProjectsResponse = self.client.get_json("agentmemory/projects").await?;
        Ok(resp
            .projects
            .into_iter()
            .map(|p| NamespaceSummary {
                namespace: p.name,
                count: p.count,
                last_updated: p.last_updated,
            })
            .collect())
    }

    async fn count(&self) -> Result<usize> {
        log::debug!("[memory::agentmemory] count");
        let resp: HealthResponse = self.client.get_json("agentmemory/health").await?;
        Ok(resp.memories.unwrap_or(0))
    }

    async fn health_check(&self) -> bool {
        let ok = self.client.livez().await;
        log::debug!("[memory::agentmemory] health_check ok={ok}");
        ok
    }
}

/// Minimal `application/x-www-form-urlencoded` style encoder for query-string
/// values. We only need to escape `/`, `?`, `#`, `&`, `=`, `+`, space, and
/// non-ASCII bytes — anything else can pass through unencoded. This avoids
/// pulling in `percent-encoding` for one call site.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// Probe whether an agentmemory daemon is reachable at the configured URL.
/// Used by the factory at startup so a `backend = "agentmemory"` config
/// against a daemon that isn't running can fail loud at boot rather than
/// silently swallow every store/recall call.
pub async fn probe_agentmemory_reachable(config: &MemoryConfig) -> Result<()> {
    let client = AgentMemoryClient::new(
        config.agentmemory_url.as_deref(),
        config.agentmemory_secret.as_deref(),
        config.agentmemory_timeout_ms,
    )?;
    if !client.livez().await {
        anyhow::bail!(
            "agentmemory daemon is not reachable at {} \
             (set MemoryConfig.backend = \"sqlite\" to fall back to the local store; \
             see https://github.com/rohitg00/agentmemory for daemon setup)",
            client.base()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_passes_through_safe_chars() {
        assert_eq!(url_encode("plain_text-123.~"), "plain_text-123.~");
    }

    #[test]
    fn url_encode_percent_escapes_specials() {
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("a/b"), "a%2Fb");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn url_encode_handles_unicode() {
        // 中文 → utf-8 bytes E4 B8 AD E6 96 87
        assert_eq!(url_encode("中文"), "%E4%B8%AD%E6%96%87");
    }
}
