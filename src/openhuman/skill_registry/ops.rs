//! Business logic for the skill registry: fetch, index, search, and install.
//!
//! The catalog is sourced from the HermesHub aggregated JSON API which
//! includes skills from HermesHub (built-in + optional), ClawHub, skills.sh,
//! LobeHub, and browse.sh — all accessible from a single endpoint.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;

use super::store;
use super::store::CachedCatalog;
use super::types::CatalogEntry;

const CATALOG_URL: &str = "https://hermes-agent.nousresearch.com/docs/api/skills.json";
const CATALOG_URL_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL";
const DOWNLOAD_BASE_URL_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL";
const REFRESH_ON_BOOT_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_REFRESH_ON_BOOT";
const FETCH_TIMEOUT_SECS: u64 = 180;

/// Single-flight gate for catalog fetches. On mount the skills explorer issues
/// several catalog reads that each funnel into [`browse_catalog`] — `sources`
/// and `browse` from two separate effects, plus `search` as the user types —
/// and React StrictMode double-invokes those effects in dev, so a handful of
/// reads land within the same instant. Without this lock each would issue its
/// own ~80s download of the same ~90k-entry catalog. Concurrent cache-miss
/// callers serialize here, and all but the first re-read the just-written cache
/// instead of hitting the network.
static FETCH_LOCK: Mutex<()> = Mutex::const_new(());

/// True while a background (stale-while-revalidate) refresh is scheduled or
/// running, so a burst of stale-cache reads spawns at most one refresh task.
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Clears [`REFRESHING`] when the background refresh task ends (incl. panic).
struct RefreshGuard;
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESHING.store(false, Ordering::Release);
    }
}

/// Start a one-shot background refresh of the remote skills catalog.
///
/// This is intended for core startup: it warms the explorer/search cache without
/// making core readiness depend on registry availability. Set
/// `OPENHUMAN_SKILL_REGISTRY_REFRESH_ON_BOOT=0` to disable it in constrained
/// environments.
pub fn start_boot_catalog_refresh() {
    static STARTED: std::sync::Once = std::sync::Once::new();

    STARTED.call_once(|| {
        if !refresh_on_boot_enabled(std::env::var(REFRESH_ON_BOOT_ENV).ok().as_deref()) {
            tracing::info!(
                env = REFRESH_ON_BOOT_ENV,
                "[skill_registry] boot catalog refresh disabled"
            );
            return;
        }

        tracing::info!("[skill_registry] scheduling boot catalog refresh");
        tokio::spawn(async {
            let started = std::time::Instant::now();
            match browse_catalog(true).await {
                Ok(entries) => {
                    tracing::info!(
                        count = entries.len(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "[skill_registry] boot catalog refresh complete"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        elapsed_ms = started.elapsed().as_millis(),
                        "[skill_registry] boot catalog refresh failed"
                    );
                }
            }
        });
    });
}

fn refresh_on_boot_enabled(raw: Option<&str>) -> bool {
    let Some(raw) = raw else { return true };
    let value = raw.trim();
    !(value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off"))
}

/// Whether a past-TTL (stale) cache may be served without a network round-trip.
#[derive(Clone, Copy, PartialEq)]
enum StaleMode {
    /// Serve stale immediately + revalidate in the background — for the
    /// unfiltered browse, where a slightly-old catalog is fine and speed wins.
    Allow,
    /// Treat stale as a miss and fetch fresh under the single-flight lock — for
    /// search / filter reads, which must reflect the current catalog.
    Reject,
}

/// Fetch the full catalog for the **unfiltered browse** view, accepting a stale
/// cache (stale-while-revalidate):
/// - **Fresh cache** → returned immediately.
/// - **Stale cache** (past TTL) → returned immediately *and* a single background
///   refresh is kicked off, so the explorer renders from the last-known catalog
///   instead of blocking on the ~80s download.
/// - **No cache** → fetch under the single-flight lock; concurrent callers
///   coalesce onto that one request.
///
/// `force_refresh == true` (boot warm-up / explicit refresh) always re-fetches.
/// Search / filter reads use [`browse_catalog_fresh`], which never serves stale.
pub async fn browse_catalog(force_refresh: bool) -> Result<Vec<CatalogEntry>, String> {
    browse_catalog_with(force_refresh, StaleMode::Allow, fetch_catalog_uncached).await
}

/// Fetch the full catalog for **search / filter** reads. Never serves a stale
/// cache: a fresh cache is used as-is, but a stale-or-absent cache falls through
/// to a (single-flight) fresh fetch so results aren't computed over an outdated
/// catalog. Thanks to single-flight, a search issued while a background
/// revalidation is already running simply awaits that in-flight fetch rather
/// than starting a new one.
pub async fn browse_catalog_fresh() -> Result<Vec<CatalogEntry>, String> {
    browse_catalog_with(false, StaleMode::Reject, fetch_catalog_uncached).await
}

/// Core of [`browse_catalog`] / [`browse_catalog_fresh`], parameterised over the
/// fetcher so the cache / single-flight orchestration can be unit-tested without
/// real network I/O.
async fn browse_catalog_with<F, Fut>(
    force_refresh: bool,
    stale_mode: StaleMode,
    fetch: F,
) -> Result<Vec<CatalogEntry>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<CatalogEntry>, String>>,
{
    if !force_refresh {
        match store::load_cached_catalog_state() {
            Some(CachedCatalog::Fresh(entries)) => {
                tracing::debug!(
                    count = entries.len(),
                    "[skill_registry] serving fresh cache"
                );
                return Ok(entries);
            }
            // Browse: serve stale now, revalidate in background.
            Some(CachedCatalog::Stale(entries)) if stale_mode == StaleMode::Allow => {
                tracing::info!(
                    count = entries.len(),
                    "[skill_registry] serving stale cache; revalidating in background"
                );
                spawn_background_refresh();
                return Ok(entries);
            }
            // Search / filter: stale is not good enough — fall through to fetch.
            Some(CachedCatalog::Stale(_)) => {
                tracing::debug!("[skill_registry] stale cache rejected for fresh read; fetching");
            }
            None => {}
        }
    }

    // Single-flight: only one fetch runs at a time. Callers that queued behind
    // the lock re-check the cache below and reuse the just-fetched result.
    let _guard = FETCH_LOCK.lock().await;
    if !force_refresh {
        if let Some(CachedCatalog::Fresh(entries)) = store::load_cached_catalog_state() {
            tracing::debug!(
                count = entries.len(),
                "[skill_registry] cache populated by concurrent fetch; reusing"
            );
            return Ok(entries);
        }
    }

    fetch().await
}

/// Spawn at most one background catalog refresh (stale-while-revalidate). Extra
/// calls while a refresh is in flight no-op via [`REFRESHING`]. The refresh runs
/// under [`FETCH_LOCK`] so it never races a foreground fetch.
fn spawn_background_refresh() {
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async {
        let _reset = RefreshGuard;
        let _guard = FETCH_LOCK.lock().await;
        match fetch_catalog_uncached().await {
            Ok(entries) => tracing::info!(
                count = entries.len(),
                "[skill_registry] background catalog refresh complete"
            ),
            Err(error) => {
                tracing::warn!(error = %error, "[skill_registry] background catalog refresh failed")
            }
        }
    });
}

/// Download, parse, index, and cache the catalog — the network path, unguarded.
/// Callers must go through [`browse_catalog_with`] / [`spawn_background_refresh`]
/// so this runs under the single-flight lock.
async fn fetch_catalog_uncached() -> Result<Vec<CatalogEntry>, String> {
    let catalog_url = catalog_url();
    tracing::info!(
        catalog_url = %redact_url_for_log(&catalog_url),
        "[skill_registry] fetching catalog"
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    let response = client
        .get(&catalog_url)
        .header("User-Agent", "openhuman-core")
        .send()
        .await
        .map_err(|e| format!("catalog fetch failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "catalog returned status {}",
            response.status().as_u16()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("failed to read response: {e}"))?;

    let raw_items: Vec<serde_json::Value> = parse_catalog_json(&body)?;

    tracing::info!(
        total_raw = raw_items.len(),
        "[skill_registry] parsing catalog"
    );

    let entries: Vec<CatalogEntry> = raw_items.iter().filter_map(parse_hermes_entry).collect();

    tracing::info!(count = entries.len(), "[skill_registry] catalog indexed");

    store::save_catalog_cache(&entries);
    Ok(entries)
}

fn catalog_url() -> String {
    std::env::var(CATALOG_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CATALOG_URL.to_string())
}

fn redact_url_for_log(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(parsed) => {
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or("");
            let path = parsed.path();
            format!("{scheme}://{host}{path}")
        }
        Err(_) => "<unparseable>".to_string(),
    }
}

pub(crate) fn parse_catalog_json(body: &str) -> Result<Vec<serde_json::Value>, String> {
    serde_json::from_str(body).map_err(|e| format!("invalid catalog json: {e}"))
}

/// Search the catalog by query string.
pub async fn search_catalog(
    query: &str,
    source_filter: Option<&str>,
    category_filter: Option<&str>,
) -> Result<Vec<CatalogEntry>, String> {
    tracing::debug!(
        query = %query,
        source_filter = ?source_filter,
        category_filter = ?category_filter,
        "[skill_registry] search_catalog"
    );
    // Search/filter must reflect the current catalog — never serve stale.
    let catalog = browse_catalog_fresh().await?;
    let q = query.to_lowercase();

    let filtered: Vec<CatalogEntry> = catalog
        .into_iter()
        .filter(|entry| {
            if let Some(src) = source_filter {
                if !entry.source.eq_ignore_ascii_case(src) {
                    return false;
                }
            }
            if let Some(cat) = category_filter {
                if !entry.category.eq_ignore_ascii_case(cat) {
                    return false;
                }
            }
            if q.is_empty() {
                return true;
            }
            entry.name.to_lowercase().contains(&q)
                || entry.description.to_lowercase().contains(&q)
                || entry.tags.iter().any(|t| t.to_lowercase().contains(&q))
                || entry.category.to_lowercase().contains(&q)
                || entry
                    .author
                    .as_deref()
                    .map(|a| a.to_lowercase().contains(&q))
                    .unwrap_or(false)
        })
        .collect();

    tracing::debug!(
        result_count = filtered.len(),
        "[skill_registry] search complete"
    );
    Ok(filtered)
}

/// Return the distinct set of upstream sources present in the catalog.
pub async fn list_sources() -> Result<Vec<String>, String> {
    let catalog = browse_catalog(false).await?;
    let mut sources: Vec<String> = catalog
        .iter()
        .map(|e| e.source.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    sources.sort();
    Ok(sources)
}

/// Return the distinct set of categories present in the catalog.
pub async fn list_categories() -> Result<Vec<String>, String> {
    let catalog = browse_catalog(false).await?;
    let mut categories: Vec<String> = catalog
        .iter()
        .map(|e| e.category.clone())
        .filter(|c| !c.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    categories.sort();
    Ok(categories)
}

/// Install a skill from the catalog by its entry id.
pub async fn install_from_catalog(
    workspace_dir: &std::path::Path,
    entry: &CatalogEntry,
) -> Result<crate::openhuman::workflows::ops_install::InstallWorkflowFromUrlOutcome, String> {
    tracing::info!(
        entry_id = %entry.id,
        source = %entry.source,
        download_url = %entry.download_url,
        "[skill_registry] installing from catalog"
    );

    if entry.download_url.trim().is_empty() {
        let where_to_find = entry
            .source_url
            .as_deref()
            .map(|u| format!(" View it at {u}."))
            .unwrap_or_default();
        return Err(format!(
            "'{}' is hosted on {} and has no direct SKILL.md download, so it can't be installed automatically yet.{}",
            entry.name, entry.source, where_to_find
        ));
    }

    let params = crate::openhuman::workflows::ops_install::InstallWorkflowFromUrlParams {
        url: entry.download_url.clone(),
        timeout_secs: Some(60),
    };

    crate::openhuman::workflows::ops_install::install_workflow_from_url(workspace_dir, params).await
}

pub(crate) fn parse_hermes_entry(item: &serde_json::Value) -> Option<CatalogEntry> {
    let name = item.get("name").and_then(|v| v.as_str())?.to_string();

    let description = item
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source = item
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("hermes")
        .to_string();

    let category = item
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let author = item
        .get("author")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let version = item
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let license = item
        .get("license")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags = item
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let platforms = item
        .get("platforms")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let commands = item
        .get("commands")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let env_vars = item
        .get("envVars")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let docs_path = item
        .get("docsPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let source_url = item
        .get("sourceUrl")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let download_url = derive_download_url(
        &source,
        &category,
        &name,
        docs_path.as_deref(),
        source_url.as_deref(),
    );

    Some(CatalogEntry {
        id: name.clone(),
        name,
        description,
        source,
        category,
        author,
        version,
        tags,
        platforms,
        download_url,
        source_url,
        docs_path,
        commands,
        env_vars,
        license,
    })
}

/// Resolve a fetchable `SKILL.md` URL for a catalog entry.
///
/// Precedence:
/// 1. `OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL` test override.
/// 2. `docsPath` — Hermes' own bundled / optional skills, which live in the
///    `NousResearch/hermes-agent` repo under `skills/` / `optional-skills/`.
/// 3. `sourceUrl` — community skills (ClawHub / LobeHub / skills.sh / browse.sh
///    / NVIDIA). When it points at a GitHub blob/tree it is rewritten to the
///    `raw.githubusercontent.com` `SKILL.md`; non-GitHub portals have no raw
///    download.
///
/// Returns an empty string when no direct download can be derived (portal-only
/// community skills). [`install_from_catalog`] turns that into an actionable
/// error rather than fetching a guaranteed-404 URL. Previously every community
/// skill was force-templated onto a `NousResearch/hermes-agent` path it never
/// lived at, so virtually all community installs 404'd — issue #3741.
fn derive_download_url(
    _source: &str,
    _category: &str,
    name: &str,
    docs_path: Option<&str>,
    source_url: Option<&str>,
) -> String {
    if let Ok(base) = std::env::var(DOWNLOAD_BASE_URL_ENV) {
        let base = base.trim().trim_end_matches('/');
        if !base.is_empty() {
            return format!("{base}/{name}/SKILL.md");
        }
    }
    if let Some(url) = docs_path.and_then(download_url_from_docs_path) {
        return url;
    }
    if let Some(url) = source_url.and_then(download_url_from_source_url) {
        return url;
    }
    // No resolvable direct download (portal-only community skill).
    String::new()
}

/// Rewrite a GitHub `sourceUrl` (blob or tree view) into the raw
/// `SKILL.md` download URL. Returns `None` for non-GitHub hosts (portal pages
/// that serve HTML, not raw markdown).
///
/// - blob: `…/github.com/{owner}/{repo}/blob/{branch}/{path}` →
///   `…/raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}`
/// - tree (directory): same rewrite, then append `/SKILL.md`.
fn download_url_from_source_url(source_url: &str) -> Option<String> {
    let rest = source_url
        .strip_prefix("https://github.com/")
        .or_else(|| source_url.strip_prefix("http://github.com/"))?;

    // {owner}/{repo}/{blob|tree}/{branch}/{path...}
    let parts: Vec<&str> = rest.splitn(5, '/').collect();
    if parts.len() < 5 {
        return None;
    }
    let (owner, repo, kind, branch, path) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
    if owner.is_empty() || repo.is_empty() || branch.is_empty() || path.is_empty() {
        return None;
    }

    let path = path.trim_end_matches('/');
    let raw = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}");
    match kind {
        // blob points directly at a file; only append SKILL.md if it isn't one.
        "blob" => {
            if raw.ends_with("/SKILL.md") || raw.ends_with(".md") {
                Some(raw)
            } else {
                Some(format!("{raw}/SKILL.md"))
            }
        }
        // tree points at a directory — the skill's SKILL.md lives inside it.
        "tree" => Some(format!("{raw}/SKILL.md")),
        _ => None,
    }
}

fn download_url_from_docs_path(docs_path: &str) -> Option<String> {
    let parts: Vec<&str> = docs_path.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let root = match parts[0] {
        "bundled" => "skills",
        "optional" => "optional-skills",
        _ => return None,
    };
    let category = parts[1];
    let prefixed_slug = parts[2];
    let skill = prefixed_slug
        .strip_prefix(&format!("{category}-"))
        .unwrap_or(prefixed_slug);
    Some(format!(
        "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/{root}/{category}/{skill}/SKILL.md"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_hermes_entry_derives_bundled_download_url_from_docs_path() {
        let item = json!({
            "name": "apple-notes",
            "description": "Manage Apple Notes",
            "category": "apple",
            "source": "built-in",
            "docsPath": "bundled/apple/apple-apple-notes",
            "tags": ["Apple"],
            "platforms": ["macos"],
            "commands": ["memo"],
            "envVars": []
        });
        let entry = parse_hermes_entry(&item).expect("entry");
        assert_eq!(
            entry.download_url,
            "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/skills/apple/apple-notes/SKILL.md"
        );
    }

    #[test]
    fn parse_hermes_entry_derives_optional_download_url_from_docs_path() {
        let item = json!({
            "name": "docker-management",
            "description": "Manage Docker",
            "category": "devops",
            "source": "optional",
            "docsPath": "optional/devops/devops-docker-management"
        });
        let entry = parse_hermes_entry(&item).expect("entry");
        assert_eq!(
            entry.download_url,
            "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/optional-skills/devops/docker-management/SKILL.md"
        );
    }

    #[test]
    fn parse_hermes_entry_derives_github_tree_source_url() {
        // NVIDIA shape: sourceUrl is a GitHub *tree* (directory) view, no
        // docsPath. The raw SKILL.md lives inside that directory. (#3741)
        let item = json!({
            "name": "aiq-deploy",
            "description": "Deploy AIQ",
            "category": "agentic-ai",
            "source": "NVIDIA",
            "docsPath": "",
            "sourceUrl": "https://github.com/NVIDIA/skills/tree/main/skills/aiq-deploy"
        });
        let entry = parse_hermes_entry(&item).expect("entry");
        assert_eq!(
            entry.download_url,
            "https://raw.githubusercontent.com/NVIDIA/skills/main/skills/aiq-deploy/SKILL.md"
        );
        assert_eq!(
            entry.source_url.as_deref(),
            Some("https://github.com/NVIDIA/skills/tree/main/skills/aiq-deploy")
        );
    }

    #[test]
    fn parse_hermes_entry_derives_github_blob_source_url() {
        // browse.sh shape: sourceUrl is a GitHub *blob* pointing straight at the
        // SKILL.md file — rewrite host to raw, keep the path. (#3741)
        let item = json!({
            "name": "account-management",
            "description": "Account mgmt",
            "category": "account-management",
            "source": "browse.sh",
            "sourceUrl": "https://github.com/browserbase/browse.sh/blob/main/skills/plugandpay.com/account-management-ic4kjh/SKILL.md"
        });
        let entry = parse_hermes_entry(&item).expect("entry");
        assert_eq!(
            entry.download_url,
            "https://raw.githubusercontent.com/browserbase/browse.sh/main/skills/plugandpay.com/account-management-ic4kjh/SKILL.md"
        );
    }

    #[test]
    fn parse_hermes_entry_leaves_portal_source_url_undownloadable() {
        // ClawHub / LobeHub / skills.sh portals serve HTML, not raw markdown —
        // no direct download. download_url is empty; source_url is preserved so
        // install can point the user at the page. (#3741)
        for url in [
            "https://clawhub.ai/skills/agentkilox-code-audit",
            "https://lobehub.com/agent/9-somboon",
            "https://skills.sh/sickn33/antigravity-awesome-skills/00-andruia-consultant",
        ] {
            let item = json!({
                "name": "portal-skill",
                "description": "x",
                "category": "other",
                "source": "ClawHub",
                "sourceUrl": url
            });
            let entry = parse_hermes_entry(&item).expect("entry");
            assert_eq!(
                entry.download_url, "",
                "portal url must not be downloadable: {url}"
            );
            assert_eq!(entry.source_url.as_deref(), Some(url));
        }
    }

    #[test]
    fn download_url_from_source_url_rejects_non_github_and_malformed() {
        assert_eq!(
            download_url_from_source_url("https://lobehub.com/agent/x"),
            None
        );
        // GitHub URL missing the branch/path tail.
        assert_eq!(
            download_url_from_source_url("https://github.com/owner/repo"),
            None
        );
        // Unknown ref kind.
        assert_eq!(
            download_url_from_source_url("https://github.com/o/r/raw/main/x"),
            None
        );
    }

    #[tokio::test]
    async fn install_from_catalog_errors_for_portal_skill_without_download() {
        // A portal-only entry (empty download_url) must fail fast with an
        // actionable message naming the source + page — never fetch a 404. (#3741)
        let tmp = tempfile::tempdir().unwrap();
        let entry = parse_hermes_entry(&json!({
            "name": "code-audit",
            "description": "x",
            "category": "other",
            "source": "ClawHub",
            "sourceUrl": "https://clawhub.ai/skills/agentkilox-code-audit"
        }))
        .expect("entry");
        assert_eq!(entry.download_url, "");

        let err = install_from_catalog(tmp.path(), &entry)
            .await
            .expect_err("portal skill cannot install");
        assert!(err.contains("ClawHub"), "names the source: {err}");
        assert!(
            err.contains("https://clawhub.ai/skills/agentkilox-code-audit"),
            "links the source page: {err}"
        );
    }

    #[test]
    fn parse_catalog_json_rejects_invalid_payloads() {
        let error = parse_catalog_json("{").expect_err("invalid json");
        assert!(error.contains("invalid catalog json"));
    }

    #[test]
    fn refresh_on_boot_enabled_defaults_on_and_accepts_common_false_values() {
        assert!(refresh_on_boot_enabled(None));
        assert!(refresh_on_boot_enabled(Some("1")));
        assert!(refresh_on_boot_enabled(Some("true")));

        assert!(!refresh_on_boot_enabled(Some("0")));
        assert!(!refresh_on_boot_enabled(Some("false")));
        assert!(!refresh_on_boot_enabled(Some(" no ")));
        assert!(!refresh_on_boot_enabled(Some("OFF")));
    }

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;

    const CACHE_DIR_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CACHE_DIR";

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::skill_registry::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn sample_entry() -> CatalogEntry {
        parse_hermes_entry(&json!({
            "name": "apple-notes",
            "description": "Manage Apple Notes",
            "category": "apple",
            "source": "built-in",
            "docsPath": "bundled/apple/apple-apple-notes"
        }))
        .expect("entry")
    }

    #[tokio::test]
    async fn fresh_cache_skips_fetch() {
        let _env = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(CACHE_DIR_ENV, tmp.path());
        store::save_catalog_cache(&[sample_entry()]);

        let called = Arc::new(AtomicBool::new(false));
        let called_in = called.clone();
        let entries = browse_catalog_with(false, StaleMode::Allow, move || async move {
            called_in.store(true, AtomicOrdering::SeqCst);
            Ok(Vec::new())
        })
        .await
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert!(
            !called.load(AtomicOrdering::SeqCst),
            "fetcher must not run when the cache is fresh"
        );

        store::clear_cache();
        std::env::remove_var(CACHE_DIR_ENV);
    }

    #[tokio::test]
    async fn concurrent_cache_miss_coalesces_to_single_fetch() {
        let _env = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(CACHE_DIR_ENV, tmp.path());
        store::clear_cache();

        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                browse_catalog_with(false, StaleMode::Allow, move || async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    // Mimic the slow upstream so the other callers queue on the
                    // single-flight lock instead of each starting a fetch.
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    let entries = vec![sample_entry()];
                    store::save_catalog_cache(&entries);
                    Ok(entries)
                })
                .await
            }));
        }

        for handle in handles {
            let entries = handle.await.unwrap().unwrap();
            assert_eq!(entries.len(), 1, "every caller receives the catalog");
        }
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            1,
            "four concurrent cache-miss callers must trigger exactly one fetch"
        );

        store::clear_cache();
        std::env::remove_var(CACHE_DIR_ENV);
    }

    /// Write a cache file with an explicit `fetched_at_epoch` (epoch 1 => stale).
    fn write_cache_at(dir: &std::path::Path, entries: Vec<CatalogEntry>, epoch: u64) {
        let cache = store::CatalogCache {
            entries,
            fetched_at_epoch: epoch,
        };
        std::fs::write(
            dir.join("cache.json"),
            serde_json::to_string(&cache).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn browse_serves_stale_without_a_foreground_fetch() {
        let _env = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(CACHE_DIR_ENV, tmp.path());
        write_cache_at(tmp.path(), vec![sample_entry()], 1); // epoch 1 => stale

        // Pin REFRESHING so the background revalidation no-ops (no real network).
        REFRESHING.store(true, AtomicOrdering::SeqCst);
        let called = Arc::new(AtomicBool::new(false));
        let called_in = called.clone();
        let entries = browse_catalog_with(false, StaleMode::Allow, move || async move {
            called_in.store(true, AtomicOrdering::SeqCst);
            Ok(Vec::new())
        })
        .await
        .unwrap();
        REFRESHING.store(false, AtomicOrdering::SeqCst);

        assert_eq!(entries.len(), 1, "browse returns the stale entry");
        assert!(
            !called.load(AtomicOrdering::SeqCst),
            "browse must serve stale without a foreground fetch"
        );

        store::clear_cache();
        std::env::remove_var(CACHE_DIR_ENV);
    }

    #[tokio::test]
    async fn search_rejects_stale_and_fetches_fresh() {
        let _env = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(CACHE_DIR_ENV, tmp.path());
        write_cache_at(tmp.path(), vec![sample_entry()], 1); // stale: 1 entry

        let called = Arc::new(AtomicBool::new(false));
        let called_in = called.clone();
        let entries = browse_catalog_with(false, StaleMode::Reject, move || async move {
            called_in.store(true, AtomicOrdering::SeqCst);
            let fresh = vec![sample_entry(), sample_entry()];
            store::save_catalog_cache(&fresh);
            Ok(fresh)
        })
        .await
        .unwrap();

        assert!(
            called.load(AtomicOrdering::SeqCst),
            "a fresh (search) read must not be satisfied by a stale cache"
        );
        assert_eq!(
            entries.len(),
            2,
            "returns the freshly fetched catalog, not the stale one"
        );

        store::clear_cache();
        std::env::remove_var(CACHE_DIR_ENV);
    }
}
