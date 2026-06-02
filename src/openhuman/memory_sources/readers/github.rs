//! GitHub repo source reader.
//!
//! Pulls **project activity** (commits, issues, PRs) from a GitHub
//! repository — not source code. Uses the `gh` CLI when available for
//! authenticated, higher-rate-limit access; falls back to the public
//! GitHub REST API for unauthenticated reads.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};
use crate::openhuman::memory_store::content::raw::RawKind;

use super::SourceReader;

const DEFAULT_BRANCH: &str = "main";

/// Cache of issue/PR data populated during `list_items` so `read_item`
/// doesn't re-fetch each one individually. The paginated list endpoints
/// already return the full body, state, labels, etc. — caching them
/// halves the API calls (from N individual fetches down to ceil(N/100)
/// paginated pages).
///
/// Keyed by `"<owner>/<repo>:<item_id>"` (e.g. `"org/repo:issue:42"`).
/// Cleared at the start of each `list_items` call for the same repo.
static LIST_CACHE: std::sync::LazyLock<Mutex<HashMap<String, CachedItem>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

enum CachedItem {
    Issue(GhIssue),
    Pr(GhPr),
}

/// Default number of items of **each** type (commits, issues, PRs) to pull
/// when the source entry doesn't override it. Tunable per-source via
/// `max_commits` / `max_issues` / `max_prs` on [`MemorySourceEntry`].
pub(crate) const DEFAULT_GITHUB_ITEM_LIMIT: u32 = 1000;

/// GitHub REST API maximum page size (`per_page`).
const GH_PAGE_SIZE: u32 = 100;

/// Hard ceiling on pagination loops so a misbehaving API (always returning a
/// full page) can never spin forever even if `max` is enormous.
const GH_MAX_PAGES: u32 = 1000;

pub struct GithubReader;

/// Parse `owner` and `repo` from a GitHub URL.
///
/// Accepts only the canonical `https://github.com/<owner>/<repo>[.git][/]`
/// shape — extra segments like `/tree/main` or `/blob/...` are rejected
/// so callers can't accidentally derive the wrong owner/repo from a
/// deep link.
pub(crate) fn parse_github_url(url: &str) -> Result<(String, String), String> {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))
        .ok_or_else(|| format!("not a GitHub URL: {url}"))?;
    let cleaned = rest.trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = cleaned.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(format!(
            "expected https://github.com/<owner>/<repo>, got: {url}"
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn gh_available() -> bool {
    std::process::Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Item types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Commit,
    Issue,
    PullRequest,
}

impl ItemKind {
    fn prefix(self) -> &'static str {
        match self {
            ItemKind::Commit => "commit",
            ItemKind::Issue => "issue",
            ItemKind::PullRequest => "pr",
        }
    }

    fn from_id(id: &str) -> Option<(Self, &str)> {
        if let Some(rest) = id.strip_prefix("commit:") {
            Some((ItemKind::Commit, rest))
        } else if let Some(rest) = id.strip_prefix("issue:") {
            Some((ItemKind::Issue, rest))
        } else if let Some(rest) = id.strip_prefix("pr:") {
            Some((ItemKind::PullRequest, rest))
        } else {
            None
        }
    }
}

// ── Raw-archive coordinates ─────────────────────────────────────────

/// Slugifiable raw-archive source id for a repo URL.
///
/// Returns `github.com/<owner>/<repo>`, which slugifies (via
/// `slugify_source_id`) to `github-com-<owner>-<repo>` so a source's
/// commits/issues/PRs land under
/// `raw/github-com-<owner>-<repo>/{commits,issues,prs}/`.
pub(crate) fn repo_archive_source_id(url: &str) -> Option<String> {
    let (owner, repo) = parse_github_url(url).ok()?;
    Some(format!("github.com/{owner}/{repo}"))
}

/// Chunk-store source id for a single repo item (dedup key).
///
/// `github:<owner>/<repo>:<item_id>` keeps per-item uniqueness for the
/// `mem_tree_ingested_sources` dedup table while the separate
/// [`repo_chunk_scope`] drives a shared directory.
pub(crate) fn chunk_source_id(url: &str, item_id: &str) -> Option<String> {
    let (owner, repo) = parse_github_url(url).ok()?;
    Some(format!("github:{owner}/{repo}:{item_id}"))
}

/// Repo-scoped chunk path scope so all items from one repo share a
/// single directory in the content store (e.g. `document/github-org-repo/`).
pub(crate) fn repo_chunk_scope(url: &str) -> Option<String> {
    let (owner, repo) = parse_github_url(url).ok()?;
    Some(format!("github:{owner}/{repo}"))
}

/// Map a [`SourceItem`] id (`commit:<sha>`, `issue:<n>`, `pr:<n>`) to its
/// raw-archive [`RawKind`] and the clean uid used as the filename suffix.
pub(crate) fn raw_archive_coords(item_id: &str) -> Option<(RawKind, String)> {
    let (kind, rest) = ItemKind::from_id(item_id)?;
    let raw_kind = match kind {
        ItemKind::Commit => RawKind::Commit,
        ItemKind::Issue => RawKind::Issue,
        ItemKind::PullRequest => RawKind::PullRequest,
    };
    Some((raw_kind, rest.to_string()))
}

// ── gh CLI helpers ──────────────────────────────────────────────────

const GH_CLI_TIMEOUT: Duration = Duration::from_secs(30);

async fn gh_json(args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        GH_CLI_TIMEOUT,
        tokio::process::Command::new("gh").args(args).output(),
    )
    .await
    .map_err(|_| format!("gh command timed out after {}s", GH_CLI_TIMEOUT.as_secs()))?
    .map_err(|e| format!("gh command failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh exited {}: {stderr}", output.status));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("gh output not utf8: {e}"))
}

// ── API fallback helpers ────────────────────────────────────────────

async fn api_get(path: &str) -> Result<String, String> {
    let url = format!("https://api.github.com{path}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("failed to build GitHub client: {e}"))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "openhuman")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API returned {status}: {body}"));
    }

    resp.text()
        .await
        .map_err(|e| format!("failed to read response: {e}"))
}

// ── Deserialization types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GhCommit {
    sha: String,
    commit: GhCommitInner,
    /// Top-level GitHub user that authored the commit (distinct from the
    /// embedded git author identity). Present when the commit author maps
    /// to a GitHub account; absent for unlinked email-only authors.
    #[serde(default)]
    author: Option<GhUser>,
}

#[derive(Debug, Deserialize)]
struct GhCommitInner {
    message: String,
    author: Option<GhAuthor>,
    committer: Option<GhAuthor>,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    name: Option<String>,
    email: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    user: Option<GhUser>,
    labels: Vec<GhLabel>,
    created_at: Option<String>,
    updated_at: Option<String>,
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GhPr {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    user: Option<GhUser>,
    labels: Vec<GhLabel>,
    created_at: Option<String>,
    updated_at: Option<String>,
    merged_at: Option<String>,
    #[serde(default)]
    comments: u64,
}

// ── Reader implementation ───────────────────────────────────────────

#[async_trait]
impl SourceReader for GithubReader {
    fn kind(&self) -> SourceKind {
        SourceKind::GithubRepo
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let url = source
            .url
            .as_deref()
            .ok_or("github source requires a url")?;
        let (owner, repo) = parse_github_url(url)?;
        let use_gh = gh_available();

        let max_commits = source.max_commits.unwrap_or(DEFAULT_GITHUB_ITEM_LIMIT);
        let max_issues = source.max_issues.unwrap_or(DEFAULT_GITHUB_ITEM_LIMIT);
        let max_prs = source.max_prs.unwrap_or(DEFAULT_GITHUB_ITEM_LIMIT);

        let cache_dir = git_cache_dir(config, &owner, &repo);

        tracing::debug!(
            owner = %owner,
            repo = %repo,
            use_gh = use_gh,
            max_commits,
            max_issues,
            max_prs,
            cache = %cache_dir.display(),
            "[memory_sources:github] listing items"
        );

        // Clear the list cache so stale data from a prior sync doesn't
        // leak into this run.
        if let Ok(mut cache) = LIST_CACHE.lock() {
            cache.clear();
        }

        let mut items = Vec::new();
        let mut errors = Vec::new();

        // Commits via local git (clone/fetch bare repo, then git log)
        match list_commits_git(&owner, &repo, max_commits, &cache_dir).await {
            Ok(commits) => items.extend(commits),
            Err(e) => {
                tracing::warn!(error = %e, "[memory_sources:github] git commit list failed, falling back to API");
                match list_commits_api(&owner, &repo, max_commits, use_gh).await {
                    Ok(commits) => items.extend(commits),
                    Err(e2) => {
                        tracing::warn!(error = %e2, "[memory_sources:github] API commit list also failed");
                        errors.push(e2);
                    }
                }
            }
        }

        // Issues and PRs via gh CLI / API (no local equivalent)
        match list_issues(&owner, &repo, max_issues, use_gh).await {
            Ok(issues) => items.extend(issues),
            Err(e) => {
                tracing::warn!(error = %e, "[memory_sources:github] failed to list issues");
                errors.push(e);
            }
        }

        match list_prs(&owner, &repo, max_prs, use_gh).await {
            Ok(prs) => items.extend(prs),
            Err(e) => {
                tracing::warn!(error = %e, "[memory_sources:github] failed to list PRs");
                errors.push(e);
            }
        }

        if items.is_empty() && !errors.is_empty() {
            return Err(format!(
                "all GitHub API calls failed: {}",
                errors.join("; ")
            ));
        }

        tracing::debug!(count = items.len(), "[memory_sources:github] found items");
        Ok(items)
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        config: &Config,
    ) -> Result<SourceContent, String> {
        let url = source
            .url
            .as_deref()
            .ok_or("github source requires a url")?;
        let (owner, repo) = parse_github_url(url)?;
        let use_gh = gh_available();

        let (kind, ref_id) =
            ItemKind::from_id(item_id).ok_or_else(|| format!("invalid item id: {item_id}"))?;

        tracing::debug!(
            item_id = %item_id,
            kind = ?kind,
            "[memory_sources:github] reading item"
        );

        match kind {
            ItemKind::Commit => {
                let cache_dir = git_cache_dir(config, &owner, &repo);
                match read_commit_git(&owner, &repo, ref_id, &cache_dir).await {
                    Ok(content) => Ok(content),
                    Err(e) => {
                        tracing::debug!(
                            sha = %ref_id,
                            error = %e,
                            "[memory_sources:github] git read_commit failed, falling back to API"
                        );
                        read_commit_api(&owner, &repo, ref_id, use_gh).await
                    }
                }
            }
            ItemKind::Issue => {
                let num: u64 = ref_id
                    .parse()
                    .map_err(|_| format!("invalid issue number: {ref_id}"))?;
                read_issue(&owner, &repo, num, use_gh).await
            }
            ItemKind::PullRequest => {
                let num: u64 = ref_id
                    .parse()
                    .map_err(|_| format!("invalid PR number: {ref_id}"))?;
                read_pr(&owner, &repo, num, use_gh).await
            }
        }
    }
}

/// Try `gh api` first, fall back to unauthenticated REST API.
async fn fetch_github(api_path: &str, use_gh: bool) -> Result<String, String> {
    if use_gh {
        match gh_json(&["api", api_path]).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    path = %api_path,
                    "[memory_sources:github] gh failed, falling back to API"
                );
            }
        }
    }
    api_get(&format!("/{api_path}")).await
}

// ── List helpers ────────────────────────────────────────────────────

/// Fetch up to `max` rows from a paginated GitHub list endpoint.
///
/// Walks `?per_page=100&page=N` until `max` rows are collected or the API
/// returns a short page (the last page). `extra_query` is appended verbatim
/// (e.g. `"state=all"`). The result is truncated to exactly `max`.
async fn fetch_all_pages<T: serde::de::DeserializeOwned>(
    owner: &str,
    repo: &str,
    resource: &str,
    extra_query: &str,
    max: u32,
    use_gh: bool,
) -> Result<Vec<T>, String> {
    let mut out: Vec<T> = Vec::new();
    let mut page = 1u32;

    while (out.len() as u32) < max && page <= GH_MAX_PAGES {
        let remaining = max - out.len() as u32;
        let per_page = remaining.min(GH_PAGE_SIZE);
        let mut path = format!("repos/{owner}/{repo}/{resource}?per_page={per_page}&page={page}");
        if !extra_query.is_empty() {
            path.push('&');
            path.push_str(extra_query);
        }

        let json_str = fetch_github(&path, use_gh).await?;
        let batch: Vec<T> = serde_json::from_str(&json_str)
            .map_err(|e| format!("parse {resource} page {page}: {e}"))?;
        let got = batch.len();
        out.extend(batch);

        // Short page ⇒ no more rows upstream.
        if got < per_page as usize {
            break;
        }
        page += 1;
    }

    out.truncate(max as usize);
    Ok(out)
}

// ── Git-based commit helpers ───────────────────────────────────────

const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_LOG_TIMEOUT: Duration = Duration::from_secs(30);

fn git_cache_dir(config: &Config, owner: &str, repo: &str) -> PathBuf {
    config
        .workspace_dir
        .join("git_cache")
        .join(owner)
        .join(format!("{repo}.git"))
}

async fn ensure_bare_clone(owner: &str, repo: &str, cache_dir: &Path) -> Result<(), String> {
    if cache_dir.join("HEAD").exists() {
        tracing::debug!(
            cache = %cache_dir.display(),
            "[memory_sources:github:git] fetching into existing bare clone"
        );
        let output = tokio::time::timeout(
            GIT_CLONE_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["fetch", "--prune", "--quiet"])
                .current_dir(cache_dir)
                .output(),
        )
        .await
        .map_err(|_| "git fetch timed out".to_string())?
        .map_err(|e| format!("git fetch failed: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git fetch exited {}: {stderr}", output.status));
        }
        return Ok(());
    }

    if let Some(parent) = cache_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create cache dir: {e}"))?;
    }

    let clone_url = format!("https://github.com/{owner}/{repo}.git");
    tracing::info!(
        url = %clone_url,
        cache = %cache_dir.display(),
        "[memory_sources:github:git] cloning bare repo"
    );

    let output = tokio::time::timeout(
        GIT_CLONE_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["clone", "--bare", "--quiet", &clone_url])
            .arg(cache_dir)
            .output(),
    )
    .await
    .map_err(|_| "git clone timed out".to_string())?
    .map_err(|e| format!("git clone failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone exited {}: {stderr}", output.status));
    }

    Ok(())
}

async fn list_commits_git(
    owner: &str,
    repo: &str,
    max: u32,
    cache_dir: &Path,
) -> Result<Vec<SourceItem>, String> {
    ensure_bare_clone(owner, repo, cache_dir).await?;

    // git log with a custom format: sha\tsubject\ttimestamp (ISO 8601)
    let output = tokio::time::timeout(
        GIT_LOG_TIMEOUT,
        tokio::process::Command::new("git")
            .args([
                "log",
                "--all",
                &format!("--max-count={max}"),
                "--format=%H\t%s\t%aI",
            ])
            .current_dir(cache_dir)
            .output(),
    )
    .await
    .map_err(|_| "git log timed out".to_string())?
    .map_err(|e| format!("git log failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log exited {}: {stderr}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<SourceItem> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            let sha = parts.first().unwrap_or(&"");
            let subject = parts.get(1).unwrap_or(&"");
            let date = parts.get(2).unwrap_or(&"");
            SourceItem {
                id: format!("commit:{sha}"),
                title: subject.to_string(),
                updated_at_ms: parse_iso_ts(date),
            }
        })
        .collect();

    tracing::debug!(
        count = items.len(),
        "[memory_sources:github:git] listed commits via local git"
    );
    Ok(items)
}

async fn read_commit_git(
    owner: &str,
    repo: &str,
    sha: &str,
    cache_dir: &Path,
) -> Result<SourceContent, String> {
    if !cache_dir.join("HEAD").exists() {
        return Err("bare clone not present".to_string());
    }

    // git show with a custom format for author, date, and full message
    let output = tokio::time::timeout(
        GIT_LOG_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["show", "--no-patch", "--format=%H%n%aN%n%aE%n%aI%n%B", sha])
            .current_dir(cache_dir)
            .output(),
    )
    .await
    .map_err(|_| "git show timed out".to_string())?
    .map_err(|e| format!("git show failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git show exited {}: {stderr}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let full_sha = lines.next().unwrap_or(sha);
    let author_name = lines.next().unwrap_or("unknown");
    let author_email = lines.next().unwrap_or("");
    let date = lines.next().unwrap_or("unknown");
    let message: String = lines.collect::<Vec<&str>>().join("\n");
    let message = message.trim();

    let title = message.lines().next().unwrap_or("").to_string();
    let author = format!("{author_name} <{author_email}>");

    let body = format!(
        "# Commit: {title}\n\n\
         **SHA:** {full_sha}\n\
         **Author:** {author}\n\
         **Date:** {date}\n\n\
         ## Message\n\n\
         {message}",
    );

    Ok(SourceContent {
        id: format!("commit:{sha}"),
        title,
        body,
        content_type: ContentType::Markdown,
        metadata: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "sha": full_sha,
            "author": author,
        }),
    })
}

// ── API-based commit helpers (fallback) ───────────────────────────

async fn list_commits_api(
    owner: &str,
    repo: &str,
    max: u32,
    use_gh: bool,
) -> Result<Vec<SourceItem>, String> {
    let commits: Vec<GhCommit> = fetch_all_pages(owner, repo, "commits", "", max, use_gh).await?;

    Ok(commits
        .into_iter()
        .map(|c| {
            let title = c.commit.message.lines().next().unwrap_or("").to_string();
            let ts = c
                .commit
                .committer
                .as_ref()
                .and_then(|a| a.date.as_deref())
                .and_then(parse_iso_ts);
            SourceItem {
                id: format!("commit:{}", c.sha),
                title,
                updated_at_ms: ts,
            }
        })
        .collect())
}

async fn list_issues(
    owner: &str,
    repo: &str,
    max: u32,
    use_gh: bool,
) -> Result<Vec<SourceItem>, String> {
    let mut out: Vec<SourceItem> = Vec::new();
    let mut page = 1u32;

    while (out.len() as u32) < max && page <= GH_MAX_PAGES {
        let path =
            format!("repos/{owner}/{repo}/issues?per_page={GH_PAGE_SIZE}&page={page}&state=all");
        let json_str = fetch_github(&path, use_gh).await?;
        let batch: Vec<GhIssue> = serde_json::from_str(&json_str)
            .map_err(|e| format!("parse issues page {page}: {e}"))?;
        let got = batch.len();

        for i in batch {
            if i.pull_request.is_some() {
                continue;
            }
            let ts = i.updated_at.as_deref().and_then(parse_iso_ts);
            let item_id = format!("issue:{}", i.number);
            let cache_key = format!("{owner}/{repo}:{item_id}");
            out.push(SourceItem {
                id: item_id,
                title: format!("#{} {}", i.number, i.title),
                updated_at_ms: ts,
            });
            if let Ok(mut cache) = LIST_CACHE.lock() {
                cache.insert(cache_key, CachedItem::Issue(i));
            }
            if out.len() as u32 >= max {
                break;
            }
        }

        if got < GH_PAGE_SIZE as usize {
            break;
        }
        page += 1;
    }

    Ok(out)
}

async fn list_prs(
    owner: &str,
    repo: &str,
    max: u32,
    use_gh: bool,
) -> Result<Vec<SourceItem>, String> {
    let prs: Vec<GhPr> = fetch_all_pages(owner, repo, "pulls", "state=all", max, use_gh).await?;

    let items: Vec<SourceItem> = prs
        .into_iter()
        .map(|p| {
            let ts = p.updated_at.as_deref().and_then(parse_iso_ts);
            let item_id = format!("pr:{}", p.number);
            let cache_key = format!("{owner}/{repo}:{item_id}");
            let item = SourceItem {
                id: item_id,
                title: format!("PR #{} {}", p.number, p.title),
                updated_at_ms: ts,
            };
            if let Ok(mut cache) = LIST_CACHE.lock() {
                cache.insert(cache_key, CachedItem::Pr(p));
            }
            item
        })
        .collect();

    Ok(items)
}

// ── Read helpers ────────────────────────────────────────────────────

async fn read_commit_api(
    owner: &str,
    repo: &str,
    sha: &str,
    use_gh: bool,
) -> Result<SourceContent, String> {
    let json_str = fetch_github(&format!("repos/{owner}/{repo}/commits/{sha}"), use_gh).await?;

    let commit: GhCommit =
        serde_json::from_str(&json_str).map_err(|e| format!("parse commit: {e}"))?;

    let author = commit
        .commit
        .author
        .as_ref()
        .map(|a| {
            format!(
                "{} <{}>",
                a.name.as_deref().unwrap_or("unknown"),
                a.email.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();

    // GitHub login of the committer, rendered as an `@handle` so the
    // entity extractor registers it as a `handle:` entity in the memory
    // tree (unique committers become first-class entities).
    let handle = commit
        .author
        .as_ref()
        .map(|u| format!("@{}", u.login))
        .unwrap_or_default();

    let date = commit
        .commit
        .committer
        .as_ref()
        .and_then(|a| a.date.as_deref())
        .unwrap_or("unknown");

    let title = commit
        .commit
        .message
        .lines()
        .next()
        .unwrap_or("")
        .to_string();

    let author_line = if handle.is_empty() {
        author.clone()
    } else {
        format!("{author} ({handle})")
    };

    let body = format!(
        "# Commit: {title}\n\n\
         **SHA:** {sha}\n\
         **Author:** {author_line}\n\
         **Date:** {date}\n\n\
         ## Message\n\n\
         {}",
        commit.commit.message,
    );

    Ok(SourceContent {
        id: format!("commit:{sha}"),
        title,
        body,
        content_type: ContentType::Markdown,
        metadata: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "sha": sha,
            "author": author,
            "author_handle": commit.author.as_ref().map(|u| u.login.clone()),
        }),
    })
}

async fn read_issue(
    owner: &str,
    repo: &str,
    number: u64,
    use_gh: bool,
) -> Result<SourceContent, String> {
    let cache_key = format!("{owner}/{repo}:issue:{number}");
    let from_cache = LIST_CACHE
        .lock()
        .ok()
        .and_then(|mut c| c.remove(&cache_key));
    let issue: GhIssue = match from_cache {
        Some(CachedItem::Issue(i)) => i,
        _ => {
            let json_str =
                fetch_github(&format!("repos/{owner}/{repo}/issues/{number}"), use_gh).await?;
            serde_json::from_str(&json_str).map_err(|e| format!("parse issue: {e}"))?
        }
    };

    let author = issue
        .user
        .as_ref()
        .map(|u| u.login.as_str())
        .unwrap_or("unknown");
    let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
    let issue_body = issue.body.as_deref().unwrap_or("");

    let mut body = format!(
        "# Issue #{number}: {title}\n\n\
         **State:** {state}\n\
         **Author:** @{author}\n\
         **Labels:** {label_str}\n\
         **Created:** {created}\n\
         **Updated:** {updated}\n\n\
         ## Description\n\n\
         {issue_body}",
        title = issue.title,
        state = issue.state,
        label_str = if labels.is_empty() {
            "none".to_string()
        } else {
            labels.join(", ")
        },
        created = issue.created_at.as_deref().unwrap_or("unknown"),
        updated = issue.updated_at.as_deref().unwrap_or("unknown"),
    );
    let comments = fetch_issue_comments(owner, repo, number, use_gh).await;
    if !comments.is_empty() {
        body.push_str("\n\n## Comments\n");
        for comment in comments {
            body.push_str(&format!(
                "\n### @{user} at {created_at}\n\n{body}\n",
                user = comment.user,
                created_at = comment.created_at,
                body = comment.body,
            ));
        }
    }

    Ok(SourceContent {
        id: format!("issue:{number}"),
        title: format!("#{number} {}", issue.title),
        body,
        content_type: ContentType::Markdown,
        metadata: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "number": number,
            "state": issue.state,
            "labels": labels,
        }),
    })
}

async fn read_pr(
    owner: &str,
    repo: &str,
    number: u64,
    use_gh: bool,
) -> Result<SourceContent, String> {
    let cache_key = format!("{owner}/{repo}:pr:{number}");
    let from_cache = LIST_CACHE
        .lock()
        .ok()
        .and_then(|mut c| c.remove(&cache_key));
    let pr: GhPr = match from_cache {
        Some(CachedItem::Pr(p)) => p,
        _ => {
            let json_str =
                fetch_github(&format!("repos/{owner}/{repo}/pulls/{number}"), use_gh).await?;
            serde_json::from_str(&json_str).map_err(|e| format!("parse PR: {e}"))?
        }
    };

    let author = pr
        .user
        .as_ref()
        .map(|u| u.login.as_str())
        .unwrap_or("unknown");
    let labels: Vec<&str> = pr.labels.iter().map(|l| l.name.as_str()).collect();
    let pr_body = pr.body.as_deref().unwrap_or("");

    let merged_str = match pr.merged_at.as_deref() {
        Some(ts) => format!("merged at {ts}"),
        None => "not merged".to_string(),
    };

    let body = format!(
        "# PR #{number}: {title}\n\n\
         **State:** {state} ({merged})\n\
         **Author:** @{author}\n\
         **Labels:** {label_str}\n\
         **Created:** {created}\n\
         **Updated:** {updated}\n\n\
         ## Description\n\n\
         {pr_body}",
        title = pr.title,
        state = pr.state,
        merged = merged_str,
        label_str = if labels.is_empty() {
            "none".to_string()
        } else {
            labels.join(", ")
        },
        created = pr.created_at.as_deref().unwrap_or("unknown"),
        updated = pr.updated_at.as_deref().unwrap_or("unknown"),
    );

    Ok(SourceContent {
        id: format!("pr:{number}"),
        title: format!("PR #{number} {}", pr.title),
        body,
        content_type: ContentType::Markdown,
        metadata: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "number": number,
            "state": pr.state,
            "merged": pr.merged_at.is_some(),
            "labels": labels,
        }),
    })
}

// ── Comment fetching ────────────────────────────────────────────────

struct IssueComment {
    user: String,
    body: String,
    created_at: String,
}

async fn fetch_issue_comments(
    owner: &str,
    repo: &str,
    number: u64,
    use_gh: bool,
) -> Vec<IssueComment> {
    #[derive(Deserialize)]
    struct RawComment {
        user: Option<GhUser>,
        body: Option<String>,
        created_at: Option<String>,
    }

    let json_str = fetch_github(
        &format!("repos/{owner}/{repo}/issues/{number}/comments?per_page=50"),
        use_gh,
    )
    .await;

    let Ok(json_str) = json_str else {
        return Vec::new();
    };

    let comments: Vec<RawComment> = serde_json::from_str(&json_str).unwrap_or_default();

    comments
        .into_iter()
        .map(|c| IssueComment {
            user: c
                .user
                .as_ref()
                .map(|u| u.login.clone())
                .unwrap_or_else(|| "unknown".into()),
            body: c.body.unwrap_or_default(),
            created_at: c.created_at.unwrap_or_else(|| "unknown".into()),
        })
        .collect()
}

// ── Utilities ───────────────────────────────────────────────────────

fn parse_iso_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Render GitHub logins as a deduped, order-preserving, space-separated
/// list of `@handle`s. Empty / `unknown` logins are skipped; an empty
/// result renders as `none`. Used so unique committers/commenters surface
/// as `handle:` entities in the memory tree.
fn unique_handles<'a>(logins: impl Iterator<Item = &'a str>) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for login in logins {
        let l = login.trim();
        if l.is_empty() || l == "unknown" {
            continue;
        }
        if seen.insert(l.to_string()) {
            out.push(format!("@{l}"));
        }
    }
    if out.is_empty() {
        "none".to_string()
    } else {
        out.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_url_extracts_owner_and_repo() {
        let (owner, repo) = parse_github_url("https://github.com/openai/tiktoken").unwrap();
        assert_eq!(owner, "openai");
        assert_eq!(repo, "tiktoken");
    }

    #[test]
    fn parse_github_url_handles_trailing_slash_and_git() {
        let (owner, repo) = parse_github_url("https://github.com/org/repo.git/").unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_url_rejects_non_repo_paths() {
        // Deep links like /tree/main must not silently extract the wrong
        // owner/repo. Bare host or non-github URLs also rejected.
        assert!(parse_github_url("https://github.com/org/repo/tree/main").is_err());
        assert!(parse_github_url("https://gitlab.com/org/repo").is_err());
        assert!(parse_github_url("https://github.com/org").is_err());
        assert!(parse_github_url("not-a-url").is_err());
    }

    #[test]
    fn item_kind_round_trips() {
        let cases = [
            ("commit:abc123", ItemKind::Commit, "abc123"),
            ("issue:42", ItemKind::Issue, "42"),
            ("pr:99", ItemKind::PullRequest, "99"),
        ];
        for (id, expected_kind, expected_ref) in cases {
            let (kind, ref_id) = ItemKind::from_id(id).unwrap();
            assert_eq!(kind, expected_kind);
            assert_eq!(ref_id, expected_ref);
        }
    }

    #[test]
    fn item_kind_rejects_invalid() {
        assert!(ItemKind::from_id("unknown:123").is_none());
        assert!(ItemKind::from_id("noprefix").is_none());
    }

    #[test]
    fn repo_archive_source_id_slugs_to_repo_folder() {
        // `github.com/<owner>/<repo>` → slugify → `github-com-<owner>-<repo>`.
        assert_eq!(
            repo_archive_source_id("https://github.com/tinyhumansai/openhuman").as_deref(),
            Some("github.com/tinyhumansai/openhuman")
        );
        assert!(repo_archive_source_id("not-a-url").is_none());
    }

    #[test]
    fn chunk_source_id_is_clean_and_per_item() {
        assert_eq!(
            chunk_source_id("https://github.com/org/repo", "commit:abc123").as_deref(),
            Some("github:org/repo:commit:abc123")
        );
        assert_eq!(
            chunk_source_id("https://github.com/org/repo", "pr:42").as_deref(),
            Some("github:org/repo:pr:42")
        );
    }

    #[test]
    fn unique_handles_dedups_and_skips_unknown() {
        assert_eq!(
            unique_handles(["alice", "bob", "alice", "unknown", ""].into_iter()),
            "@alice @bob"
        );
        assert_eq!(unique_handles(["unknown", ""].into_iter()), "none");
        assert_eq!(unique_handles(std::iter::empty()), "none");
    }

    #[test]
    fn raw_archive_coords_maps_kind_and_uid() {
        assert_eq!(
            raw_archive_coords("commit:deadbeef"),
            Some((RawKind::Commit, "deadbeef".to_string()))
        );
        assert_eq!(
            raw_archive_coords("issue:7"),
            Some((RawKind::Issue, "7".to_string()))
        );
        assert_eq!(
            raw_archive_coords("pr:99"),
            Some((RawKind::PullRequest, "99".to_string()))
        );
        assert!(raw_archive_coords("bogus:1").is_none());
    }
}
