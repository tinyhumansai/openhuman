//! GitHub provider — incremental sync of issues and pull requests involving
//! the authenticated user, with per-item persistence into the Memory Tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Resolve the authenticated user's GitHub login (used in the search
//!      query); cached cheaply across re-fetches.
//!   4. Search for issues and PRs involving the user via
//!      `GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` with `involves:{login}`, filtered to items
//!      updated since the cursor (when available).
//!   5. For each result, persist as a single memory document if it's new
//!      *or* edited since the last sync.
//!   6. Advance the cursor to the newest `updated_at` seen and save.
//!
//! Privacy posture: the `involves:` search qualifier returns only items the
//! user created, was assigned to, mentioned in, or commented on — it never
//! surfaces private repos the user can't access. This mirrors the
//! "fetch-what-the-user-sees" model gmail / notion already follow.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use super::source::run_github_sync;
use super::sync;
use crate::openhuman::memory_sync::composio::providers::{
    merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider, CuratedTool,
    GithubFetchMode, NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    TaskFetchFilter, TaskKind,
};

pub(crate) const ACTION_GET_AUTHENTICATED_USER: &str = "GITHUB_GET_THE_AUTHENTICATED_USER";
pub(crate) const ACTION_SEARCH_ISSUES: &str = "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS";

const GH_CLI_TIMEOUT: Duration = Duration::from_secs(30);
const GITHUB_TASK_SEARCH_TIMEOUT: Duration = Duration::from_secs(20);

pub struct GitHubProvider;

impl GitHubProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GitHubProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for GitHubProvider {
    fn toolkit_slug(&self) -> &'static str {
        "github"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::GITHUB_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 30 minutes — GitHub issues change less frequently than Slack
        // messages, so a half-hour cadence keeps the memory fresh without
        // hammering the search API.
        Some(resolve_sync_interval_secs("github", 30 * 60))
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:github] fetch_user_profile via {ACTION_GET_AUTHENTICATED_USER}"
        );

        let resp = ctx
            .execute(ACTION_GET_AUTHENTICATED_USER, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:github] {ACTION_GET_AUTHENTICATED_USER} failed: {e:#}")
            })?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:github] {ACTION_GET_AUTHENTICATED_USER}: {err}"
            ));
        }

        let data = &resp.data;
        let login = sync::extract_user_login(data);
        let display_name = pick_str(data, &["name", "data.name"]).or_else(|| login.clone());
        let email = pick_str(data, &["email", "data.email"]);
        let avatar_url = pick_str(data, &["avatar_url", "data.avatar_url"]);
        let profile_url = pick_str(data, &["html_url", "data.html_url"]);

        Ok(ProviderUserProfile {
            toolkit: "github".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username: login,
            avatar_url,
            profile_url,
            extras: data.clone(),
        })
    }

    /// Incremental sync via the generic
    /// [`orchestrator`](crate::openhuman::memory_sync::composio::providers::orchestrator):
    /// login resolution, pagination, dedup, the `max_items` cap, and cursor
    /// handling live in `run_sync`; the GitHub-specific primitives — including
    /// the **server-side** `sync_depth_days` window — live in [`super::source`].
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        run_github_sync(ctx, reason).await
    }

    async fn fetch_tasks(
        &self,
        ctx: &ProviderContext,
        filter: &TaskFetchFilter,
    ) -> Result<Vec<NormalizedTask>, String> {
        let max = filter.effective_max();
        let query = build_fetch_query(filter);
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            max,
            mode = ?filter.github_fetch_mode,
            query = %query,
            "[composio:github] fetch_tasks"
        );

        // Select the data source by the user-configured fetch mode. `Auto`
        // (the default) keeps the shipped Composio path as primary and treats
        // local `gh`/REST as a true fallback — only used when the Composio
        // round-trip errors or is unavailable. `Composio` / `Local` force one
        // path. Normalization happens ONCE below regardless of source.
        let data = match filter.github_fetch_mode {
            GithubFetchMode::Composio => {
                fetch_github_tasks_composio(ctx, &query, max, &filter.extra).await?
            }
            GithubFetchMode::Local => fetch_github_tasks_local(&query, max, &filter.extra).await?,
            GithubFetchMode::Auto => {
                match fetch_github_tasks_composio(ctx, &query, max, &filter.extra).await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::info!(
                            error = %e,
                            "[composio:github] Composio fetch unavailable; falling back to local gh/REST"
                        );
                        fetch_github_tasks_local(&query, max, &filter.extra).await?
                    }
                }
            }
        };

        let mut out: Vec<NormalizedTask> = Vec::new();
        for issue in sync::extract_issues(&data) {
            if out.len() >= max {
                break;
            }
            if let Some(nt) = normalize_github_issue(&issue) {
                out.push(nt);
            }
        }
        tracing::debug!(count = out.len(), "[composio:github] fetch_tasks complete");
        Ok(out)
    }
}

/// Fetch GitHub issues/PRs through the connected Composio account.
///
/// This is the original shipped `fetch_tasks` data path: it builds the
/// `GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` search args, merges any advanced
/// `extra` query fragment, fires the action through the mode-aware
/// `ctx.execute` chokepoint, and returns the raw response `data` for the
/// shared normalization loop. Kept as a sibling of
/// [`fetch_github_tasks_local`] so `fetch_tasks` can select between them by
/// [`GithubFetchMode`].
async fn fetch_github_tasks_composio(
    ctx: &ProviderContext,
    query: &str,
    max: usize,
    extra: &Value,
) -> Result<Value, String> {
    let mut args = json!({
        "q": query,
        "sort": "updated",
        "order": "desc",
        "per_page": max.min(100) as u32,
        "page": 1,
    });
    merge_extra(&mut args, extra);

    let resp = ctx
        .execute(ACTION_SEARCH_ISSUES, Some(args))
        .await
        .map_err(|e| format!("[composio:github] {ACTION_SEARCH_ISSUES}: {e:#}"))?;
    if !resp.successful {
        return Err(format!(
            "[composio:github] {ACTION_SEARCH_ISSUES}: {}",
            resp.error.unwrap_or_else(|| "provider failure".into())
        ));
    }
    Ok(resp.data)
}

async fn fetch_github_tasks_local(query: &str, max: usize, extra: &Value) -> Result<Value, String> {
    let mut args = json!({
        "q": query,
        "sort": "updated",
        "order": "desc",
        "per_page": max.min(100) as u32,
        "page": 1,
    });
    merge_extra(&mut args, extra);
    expand_me_in_github_search_args(&mut args).await;

    match gh_search_issues(&args).await {
        Ok(data) => Ok(data),
        Err(gh_err) => {
            tracing::debug!(
                error = %gh_err,
                "[task_sources:github] gh api search failed, falling back to REST"
            );
            rest_search_issues(&args).await.map_err(|rest_err| {
                format!("[task_sources:github] local GitHub search failed: gh: {gh_err}; REST: {rest_err}")
            })
        }
    }
}

async fn gh_search_issues(args: &Value) -> Result<Value, String> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.arg("api")
        .arg("--method")
        .arg("GET")
        .arg("search/issues");
    for (key, value) in github_search_arg_pairs(args)? {
        cmd.arg("-f").arg(format!("{key}={value}"));
    }

    let output = tokio::time::timeout(GH_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("gh command timed out after {}s", GH_CLI_TIMEOUT.as_secs()))?
        .map_err(|e| format!("gh command failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh exited {}: {stderr}", output.status));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("gh output not utf8: {e}"))?;
    serde_json::from_str(&stdout).map_err(|e| format!("parse gh search response: {e}"))
}

async fn rest_search_issues(args: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(GITHUB_TASK_SEARCH_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build GitHub client: {e}"))?;

    let mut request = client
        .get("https://api.github.com/search/issues")
        .header("User-Agent", "openhuman")
        .header("Accept", "application/vnd.github+json");

    if let Some(token) = github_env_token() {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let pairs = github_search_arg_pairs(args)?;
    let resp = request
        .query(&pairs)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API returned {status}: {body}"));
    }

    resp.json::<Value>()
        .await
        .map_err(|e| format!("parse GitHub API response: {e}"))
}

pub(super) fn github_env_token() -> Option<String> {
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn expand_me_in_github_search_args(args: &mut Value) {
    let Some(query) = args.get("q").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    if !query.contains("@me") {
        return;
    }
    let Some(login) = resolve_github_login().await else {
        return;
    };
    if let Some(obj) = args.as_object_mut() {
        obj.insert("q".to_string(), Value::String(query.replace("@me", &login)));
    }
}

async fn resolve_github_login() -> Option<String> {
    if let Some(login) = resolve_github_login_with_gh().await {
        return Some(login);
    }
    resolve_github_login_with_rest().await
}

async fn resolve_github_login_with_gh() -> Option<String> {
    let output = tokio::time::timeout(
        GH_CLI_TIMEOUT,
        tokio::process::Command::new("gh")
            .arg("api")
            .arg("user")
            .arg("--jq")
            .arg(".login")
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn resolve_github_login_with_rest() -> Option<String> {
    let token = github_env_token()?;
    let client = reqwest::Client::builder()
        .timeout(GITHUB_TASK_SEARCH_TIMEOUT)
        .build()
        .ok()?;
    let resp = client
        .get("https://api.github.com/user")
        .header("User-Agent", "openhuman")
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("login")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(super) fn github_search_arg_pairs(args: &Value) -> Result<Vec<(String, String)>, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "GitHub search args must be a JSON object".to_string())?;
    let mut out = Vec::with_capacity(obj.len());
    for (key, value) in obj {
        let rendered = match value {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => continue,
            other => other.to_string(),
        };
        if !rendered.is_empty() {
            out.push((key.clone(), rendered));
        }
    }
    Ok(out)
}

/// Build a GitHub Search-Issues query from a [`TaskFetchFilter`].
///
/// Combines repo / label / state / assignee qualifiers. When the filter
/// carries no scoping constraints at all we fall back to `involves:@me` so a
/// task source never accidentally pulls the entire public issue universe.
///
/// State bias: when the filter sets no explicit `state`, we append `is:open`
/// so closed issues and merged/closed PRs aren't fetched in the first place
/// (the unconditional skip in `normalize_github_issue` is the hard guarantee;
/// this is the fetch-side optimization). An explicit `state` is respected and
/// `is:open` is not double-added.
pub(super) fn build_fetch_query(filter: &TaskFetchFilter) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(repo) = filter
        .repo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_github_repo_filter)
    {
        parts.push(format!("repo:{repo}"));
    }
    for label in filter
        .labels
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
    {
        parts.push(format!("label:\"{label}\""));
    }
    if filter.assignee_is_me {
        parts.push("assignee:@me".to_string());
    }
    // If no repo/label/assignee scoping was supplied, fall back to
    // `involves:@me` (plus the open bias) rather than the whole issue universe.
    if parts.is_empty() {
        parts.push("involves:@me".to_string());
    }
    let explicit_state = filter
        .state
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match explicit_state {
        // Caller pinned a state — respect it verbatim, don't add `is:open`.
        Some(state) => parts.push(format!("state:{state}")),
        // No explicit state — bias the fetch toward open items.
        None => parts.push("is:open".to_string()),
    }
    parts.join(" ")
}

pub(super) fn normalize_github_repo_filter(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))
        .unwrap_or(trimmed);
    let cleaned = without_scheme
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let mut parts = cleaned.split('/').filter(|part| !part.is_empty());
    match (parts.next(), parts.next()) {
        (Some(owner), Some(repo)) => {
            let repo = repo.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() {
                trimmed.to_string()
            } else {
                format!("{owner}/{repo}")
            }
        }
        _ => trimmed.to_string(),
    }
}

/// Map a raw GitHub issue/PR payload into a [`NormalizedTask`].
///
/// GitHub's search-issues-and-PRs endpoint returns both shapes; a hit is a
/// pull request iff it carries a `pull_request` object. We tag the kind here
/// so enrichment can phrase the objective as "review" vs "resolve".
///
/// Returns `None` when the item's state is `"closed"` — a merged/closed PR
/// and a closed issue both report `state == "closed"`, and there is no point
/// ingesting work that is already done. This skip is unconditional (it does
/// not depend on the fetch query), so even if a `closed` item slips through
/// the query bias it is dropped here.
pub(super) fn normalize_github_issue(issue: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id = sync::extract_issue_id(issue)?;
    let status = pick_str(issue, &["state", "data.state"]);
    if status
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("closed"))
        .unwrap_or(false)
    {
        tracing::debug!(
            external_id = %external_id,
            "[composio:github] normalize_github_issue: skipping closed item (merged PR / closed issue)"
        );
        return None;
    }
    let title =
        sync::extract_issue_title(issue).unwrap_or_else(|| format!("GitHub issue {external_id}"));
    let kind = if is_pull_request(issue) {
        TaskKind::PullRequest
    } else {
        TaskKind::Issue
    };
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "github".to_string(),
        kind,
        title,
        body: pick_str(issue, &["body", "data.body"]),
        url: pick_str(issue, &["html_url", "data.html_url"]),
        status,
        assignee: pick_str(issue, &["assignee.login", "data.assignee.login"]),
        due: None,
        labels: extract_github_labels(issue),
        priority: None,
        updated_at: sync::extract_issue_updated_at(issue),
        raw: issue.clone(),
    })
}

/// A GitHub search hit is a pull request iff it carries a non-null
/// `pull_request` object (issues never do). Tolerant of the Composio `data`
/// wrapper.
fn is_pull_request(issue: &serde_json::Value) -> bool {
    let pr = issue
        .get("pull_request")
        .or_else(|| issue.get("data").and_then(|d| d.get("pull_request")));
    matches!(pr, Some(v) if !v.is_null())
}

/// Extract label names from a GitHub issue payload (`labels` is an array
/// of `{ name }` objects). Tolerant of the Composio `data` wrapper.
fn extract_github_labels(issue: &serde_json::Value) -> Vec<String> {
    let arr = issue
        .get("labels")
        .or_else(|| issue.get("data").and_then(|d| d.get("labels")))
        .and_then(|v| v.as_array());
    match arr {
        Some(items) => items
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

///
/// `involves:` is GitHub's logical-OR over `author`, `assignee`, `mentions`,
/// and `commenter`, so the result set covers every item the connected user
/// has standing in — not only items explicitly assigned to them. When a
/// cursor from a prior sync is present, an `updated:>{cursor}` clause is
/// appended so the next page request only returns items changed since.
///
/// Kept as a free function (rather than inline in `sync()`) so the query
/// contract — specifically the `involves:` qualifier — can be asserted by
/// unit tests without spinning up the full sync pipeline.
pub(super) fn build_search_query(login: &str, cursor: Option<&str>) -> String {
    match cursor {
        Some(cursor) => format!("involves:{login} updated:>{cursor}"),
        None => format!("involves:{login}"),
    }
}

/// Extended variant that optionally appends a `sync_depth_days` fragment on
/// first sync (no cursor). The `depth_fragment` is expected to be a pre-built
/// `"updated:>{date}"` string.
pub(super) fn build_search_query_with_depth(
    login: &str,
    cursor: Option<&str>,
    depth_fragment: Option<&str>,
) -> String {
    match cursor {
        Some(c) => format!("involves:{login} updated:>{c}"),
        None => match depth_fragment {
            Some(fragment) => format!("involves:{login} {fragment}"),
            None => format!("involves:{login}"),
        },
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;

    #[test]
    fn build_search_query_with_depth_no_cursor_no_depth() {
        let q = build_search_query_with_depth("alice", None, None);
        assert_eq!(q, "involves:alice");
    }

    #[test]
    fn build_search_query_with_depth_no_cursor_with_depth() {
        let q = build_search_query_with_depth("alice", None, Some("updated:>2024-01-01T00:00:00Z"));
        assert_eq!(q, "involves:alice updated:>2024-01-01T00:00:00Z");
    }

    #[test]
    fn build_search_query_with_depth_cursor_wins_over_depth() {
        // When cursor is set, depth fragment is ignored.
        let q = build_search_query_with_depth(
            "alice",
            Some("2024-06-01T00:00:00Z"),
            Some("updated:>2024-01-01T00:00:00Z"),
        );
        assert_eq!(q, "involves:alice updated:>2024-06-01T00:00:00Z");
    }
}
