//! GitHub repo source reader.
//!
//! Pulls **project activity** (commits, issues, PRs) from a GitHub
//! repository — not source code. Uses the `gh` CLI when available for
//! authenticated, higher-rate-limit access; falls back to the public
//! GitHub REST API for unauthenticated reads.

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

const DEFAULT_BRANCH: &str = "main";

pub struct GithubReader;

/// Parse `owner` and `repo` from a GitHub URL.
///
/// Accepts only the canonical `https://github.com/<owner>/<repo>[.git][/]`
/// shape — extra segments like `/tree/main` or `/blob/...` are rejected
/// so callers can't accidentally derive the wrong owner/repo from a
/// deep link.
fn parse_github_url(url: &str) -> Result<(String, String), String> {
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
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
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let url = source
            .url
            .as_deref()
            .ok_or("github source requires a url")?;
        let (owner, repo) = parse_github_url(url)?;
        let use_gh = gh_available();

        tracing::debug!(
            owner = %owner,
            repo = %repo,
            use_gh = use_gh,
            "[memory_sources:github] listing items"
        );

        let mut items = Vec::new();
        let mut errors = Vec::new();

        // Commits (last 30)
        match list_commits(&owner, &repo, use_gh).await {
            Ok(commits) => items.extend(commits),
            Err(e) => {
                tracing::warn!(error = %e, "[memory_sources:github] failed to list commits");
                errors.push(e);
            }
        }

        // Issues (last 30 open + recently closed)
        match list_issues(&owner, &repo, use_gh).await {
            Ok(issues) => items.extend(issues),
            Err(e) => {
                tracing::warn!(error = %e, "[memory_sources:github] failed to list issues");
                errors.push(e);
            }
        }

        // Pull requests (last 30)
        match list_prs(&owner, &repo, use_gh).await {
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
        _config: &Config,
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
            ItemKind::Commit => read_commit(&owner, &repo, ref_id, use_gh).await,
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

async fn list_commits(owner: &str, repo: &str, use_gh: bool) -> Result<Vec<SourceItem>, String> {
    let json_str =
        fetch_github(&format!("repos/{owner}/{repo}/commits?per_page=30"), use_gh).await?;

    // Try parsing as array of commit objects
    let commits: Vec<GhCommit> =
        serde_json::from_str(&json_str).map_err(|e| format!("parse commits: {e}"))?;

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

async fn list_issues(owner: &str, repo: &str, use_gh: bool) -> Result<Vec<SourceItem>, String> {
    let json_str = fetch_github(
        &format!("repos/{owner}/{repo}/issues?per_page=30&state=all"),
        use_gh,
    )
    .await?;

    let issues: Vec<GhIssue> =
        serde_json::from_str(&json_str).map_err(|e| format!("parse issues: {e}"))?;

    Ok(issues
        .into_iter()
        .filter(|i| i.pull_request.is_none()) // filter out PRs from issues endpoint
        .map(|i| {
            let ts = i.updated_at.as_deref().and_then(parse_iso_ts);
            SourceItem {
                id: format!("issue:{}", i.number),
                title: format!("#{} {}", i.number, i.title),
                updated_at_ms: ts,
            }
        })
        .collect())
}

async fn list_prs(owner: &str, repo: &str, use_gh: bool) -> Result<Vec<SourceItem>, String> {
    let json_str = fetch_github(
        &format!("repos/{owner}/{repo}/pulls?per_page=30&state=all"),
        use_gh,
    )
    .await?;

    let prs: Vec<GhPr> = serde_json::from_str(&json_str).map_err(|e| format!("parse PRs: {e}"))?;

    Ok(prs
        .into_iter()
        .map(|p| {
            let ts = p.updated_at.as_deref().and_then(parse_iso_ts);
            SourceItem {
                id: format!("pr:{}", p.number),
                title: format!("PR #{} {}", p.number, p.title),
                updated_at_ms: ts,
            }
        })
        .collect())
}

// ── Read helpers ────────────────────────────────────────────────────

async fn read_commit(
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

    let body = format!(
        "# Commit: {title}\n\n\
         **SHA:** {sha}\n\
         **Author:** {author}\n\
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
        }),
    })
}

async fn read_issue(
    owner: &str,
    repo: &str,
    number: u64,
    use_gh: bool,
) -> Result<SourceContent, String> {
    let json_str = fetch_github(&format!("repos/{owner}/{repo}/issues/{number}"), use_gh).await?;

    let issue: GhIssue =
        serde_json::from_str(&json_str).map_err(|e| format!("parse issue: {e}"))?;

    let author = issue
        .user
        .as_ref()
        .map(|u| u.login.as_str())
        .unwrap_or("unknown");
    let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
    let issue_body = issue.body.as_deref().unwrap_or("");

    // Fetch comments
    let comments = fetch_issue_comments(owner, repo, number, use_gh).await;

    let mut body = format!(
        "# Issue #{number}: {title}\n\n\
         **State:** {state}\n\
         **Author:** {author}\n\
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

    if !comments.is_empty() {
        body.push_str("\n\n## Comments\n");
        for comment in &comments {
            body.push_str(&format!(
                "\n### {} ({})\n\n{}\n",
                comment.user, comment.created_at, comment.body
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
    let json_str = fetch_github(&format!("repos/{owner}/{repo}/pulls/{number}"), use_gh).await?;

    let pr: GhPr = serde_json::from_str(&json_str).map_err(|e| format!("parse PR: {e}"))?;

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

    // Fetch review comments
    let comments = fetch_issue_comments(owner, repo, number, use_gh).await;

    let mut body = format!(
        "# PR #{number}: {title}\n\n\
         **State:** {state} ({merged})\n\
         **Author:** {author}\n\
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

    if !comments.is_empty() {
        body.push_str("\n\n## Comments\n");
        for comment in &comments {
            body.push_str(&format!(
                "\n### {} ({})\n\n{}\n",
                comment.user, comment.created_at, comment.body
            ));
        }
    }

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
}
