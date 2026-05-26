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
//!      `GITHUB_SEARCH_ISSUES` with `involves:{login}`, filtered to items
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
use serde_json::json;

use super::sync;
use crate::openhuman::memory_sync::composio::providers::sync_state::{
    persist_single_item, SyncState,
};
use crate::openhuman::memory_sync::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};

pub(crate) const ACTION_GET_AUTHENTICATED_USER: &str = "GITHUB_GET_AUTHENTICATED_USER";
pub(crate) const ACTION_SEARCH_ISSUES: &str = "GITHUB_SEARCH_ISSUES";

/// Items per search page on steady-state syncs.
const PAGE_SIZE: u32 = 50;

/// Larger page for the initial post-OAuth backfill.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages per sync pass. Caps initial-backfill churn; the rest rolls
/// over to the next scheduled interval.
const MAX_PAGES: u32 = 20;

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
        Some(30 * 60)
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

    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        let started_at_ms = sync::now_ms();
        let connection_id = ctx
            .connection_id
            .clone()
            .unwrap_or_else(|| "default".to_string());

        tracing::info!(
            connection_id = %connection_id,
            reason = reason.as_str(),
            "[composio:github] incremental sync starting"
        );

        // ── Step 1: load persistent sync state ──────────────────────
        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:github] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "github", &connection_id).await?;

        // ── Step 2: check daily budget ───────────────────────────────
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:github] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "github".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "github sync skipped: daily budget exhausted".to_string(),
                details: json!({ "budget_exhausted": true }),
            });
        }

        // ── Step 3: resolve the authenticated user's login ──────────
        let login = match self.resolve_login(ctx, &mut state).await {
            Ok(l) => l,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:github] budget exhausted after login probe, skipping sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "github".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "github sync skipped: daily budget exhausted after login probe"
                    .to_string(),
                details: json!({ "budget_exhausted": true, "login_resolved": true }),
            });
        }

        // ── Step 4: paginated issue search ───────────────────────────
        //
        // `involves:{login}` matches issues/PRs the user created, was assigned
        // to, was mentioned in, or commented on — scoped to what GitHub's own
        // access rules allow. Combined with `updated:>{cursor}` on subsequent
        // runs this converges on a minimal diff fetch.
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        // Build the base search query.
        let query = build_search_query(&login, state.cursor.as_deref());

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_updated: Option<String> = None;

        'pages: for page_num in 1..=MAX_PAGES {
            if state.budget_exhausted() {
                tracing::info!(
                    page = page_num,
                    "[composio:github] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            let args = json!({
                "q": query,
                "sort": "updated",
                "order": "desc",
                "per_page": page_size,
                "page": page_num,
            });

            tracing::debug!(
                connection_id = %connection_id,
                page = page_num,
                query = %query,
                "[composio:github] executing {ACTION_SEARCH_ISSUES}"
            );

            let resp = match ctx.execute(ACTION_SEARCH_ISSUES, Some(args)).await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = state.save(&memory).await;
                    return Err(format!(
                        "[composio:github] {ACTION_SEARCH_ISSUES} page={page_num}: {e:#}"
                    ));
                }
            };
            state.record_requests(1);

            if !resp.successful {
                let err = resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "provider reported failure".to_string());
                let _ = state.save(&memory).await;
                return Err(format!(
                    "[composio:github] {ACTION_SEARCH_ISSUES} page={page_num}: {err}"
                ));
            }

            let issues = sync::extract_issues(&resp.data);
            total_fetched += issues.len();

            if issues.is_empty() {
                tracing::debug!(
                    page = page_num,
                    "[composio:github] empty page, stopping pagination"
                );
                break;
            }

            // ── Per-item dedup + persist ─────────────────────────────
            for issue in &issues {
                let Some(issue_id) = sync::extract_issue_id(issue) else {
                    tracing::debug!("[composio:github] issue missing id, skipping");
                    continue;
                };

                let updated = sync::extract_issue_updated_at(issue);

                // Track the newest `updated_at` for cursor advancement.
                if let Some(ref ts) = updated {
                    if newest_updated.as_ref().is_none_or(|ex| ts > ex) {
                        newest_updated = Some(ts.clone());
                    }
                }

                // Composite dedup key: issue_id@updated_at (same trick ClickUp
                // uses so that edits after the last sync are re-persisted).
                let sync_key = match &updated {
                    Some(ts) => format!("{issue_id}@{ts}"),
                    None => issue_id.clone(),
                };

                // If the item's updated_at is at or before our cursor AND we've
                // already synced this composite key, every subsequent result on
                // this page is guaranteed to be older — stop pagination early.
                if let (Some(ref cursor), Some(ref ts)) = (&state.cursor, &updated) {
                    if ts <= cursor && state.is_synced(&sync_key) {
                        tracing::debug!(
                            issue_id = %issue_id,
                            "[composio:github] reached cursor boundary, stopping"
                        );
                        break 'pages;
                    }
                }

                if state.is_synced(&sync_key) {
                    continue;
                }

                let title_text = sync::extract_issue_title(issue)
                    .unwrap_or_else(|| format!("GitHub issue {issue_id}"));
                let doc_id = format!("composio-github-issue-{issue_id}");

                match persist_single_item(
                    &memory,
                    "github",
                    &doc_id,
                    &title_text,
                    issue,
                    "github",
                    ctx.connection_id.as_deref(),
                )
                .await
                {
                    Ok(_) => {
                        state.mark_synced(&sync_key);
                        total_persisted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            issue_id = %issue_id,
                            error = %e,
                            "[composio:github] failed to persist issue (continuing)"
                        );
                    }
                }
            }

            // GitHub search pages are 0-indexed in terms of total results;
            // a short page means we've exhausted the result set.
            if (issues.len() as u32) < page_size {
                tracing::debug!(
                    page = page_num,
                    returned = issues.len(),
                    "[composio:github] short page, end of results"
                );
                break;
            }
        }

        // ── Step 5: advance cursor and save state ────────────────────
        if let Some(new_cursor) = newest_updated {
            state.advance_cursor(&new_cursor);
        }
        state.set_last_sync_at_ms(sync::now_ms());
        state.save(&memory).await?;

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "github sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
             budget remaining {remaining}",
            reason = reason.as_str(),
            remaining = state.budget_remaining(),
        );
        tracing::info!(
            connection_id = %connection_id,
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            total_fetched,
            total_persisted,
            budget_remaining = state.budget_remaining(),
            "[composio:github] incremental sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "github".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_persisted,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "issues_fetched": total_fetched,
                "issues_persisted": total_persisted,
                "budget_remaining": state.budget_remaining(),
                "cursor": state.cursor,
                "synced_ids_total": state.synced_ids.len(),
            }),
        })
    }
}

impl GitHubProvider {
    /// Resolve the authenticated user's GitHub login handle.
    ///
    /// The login is stable for the connection lifetime. We re-fetch on every
    /// sync rather than caching in `SyncState` to (a) keep the struct lean
    /// and (b) implicitly validate that the OAuth token is still valid before
    /// we start paginating search results.
    async fn resolve_login(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        let resp = ctx
            .execute(ACTION_GET_AUTHENTICATED_USER, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:github] {ACTION_GET_AUTHENTICATED_USER} failed: {e:#}")
            })?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:github] {ACTION_GET_AUTHENTICATED_USER}: {err}"
            ));
        }

        sync::extract_user_login(&resp.data).ok_or_else(|| {
            "[composio:github] GITHUB_GET_AUTHENTICATED_USER returned no login".to_string()
        })
    }
}

/// Build the GitHub Search-Issues query for an incremental sync.
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
