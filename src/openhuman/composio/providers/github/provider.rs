//! GitHub provider — incremental sync of issues assigned to the
//! authenticated user, with per-item persistence into the Memory Tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Resolve the authenticated viewer's login via
//!      `GITHUB_GET_AUTHENTICATED_USER`; re-check the budget after
//!      the probe so the next call honours a tight cap strictly.
//!   4. Page through `GITHUB_SEARCH_ISSUES` with the query
//!      `assignee:<viewer_login> sort:updated-desc`. GitHub's
//!      Search API caps results at 1000, so pagination is naturally
//!      bounded. Stop each pass early once issues older than the
//!      stored cursor are reached.
//!   5. For each issue, persist as a single memory document if it's
//!      new *or* edited since the last sync.
//!   6. Advance the cursor to the newest `updated_at` seen and save.
//!
//! Privacy posture: only issues where the connected user is the
//! assignee are pulled, never the whole watched-repos issue graph.
//! Mirrors the "fetch-what-the-user-sees" model gmail / notion /
//! clickup / linear already follow. The `assignee:<login>` qualifier
//! is constructed inside the provider — never accepted from a caller
//! — so the boundary can't be tunnelled around.

use async_trait::async_trait;
use serde_json::json;

use super::sync;
use crate::openhuman::composio::providers::sync_state::{persist_single_item, SyncState};
use crate::openhuman::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};

pub(crate) const ACTION_GET_AUTHENTICATED_USER: &str = "GITHUB_GET_AUTHENTICATED_USER";
pub(crate) const ACTION_SEARCH_ISSUES: &str = "GITHUB_SEARCH_ISSUES";

/// Page size per API call. GitHub Search caps `per_page` at 100; we
/// stick with a smaller window on steady-state syncs to keep
/// response sizes bounded.
const PAGE_SIZE: u32 = 50;

/// Larger initial-sync page size, used immediately after OAuth so the
/// first backfill catches up faster.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages per sync pass before yielding. GitHub Search returns
/// at most 1000 results total, so the practical cap is 10 pages of
/// 100 — this 20 is a safety upper bound covering smaller page sizes.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Paths for extracting an issue's global integer id. GitHub's `id`
/// is unique across all of GitHub, so it's the stable key for the
/// memory document. Composio sometimes wraps the upstream payload
/// under `data`, so we check both shapes.
const ISSUE_ID_PATHS: &[&str] = &["id", "data.id"];

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
        // 30 minutes — same cadence as notion / clickup / linear.
        // GitHub Search has stricter per-minute rate limits than the
        // core API but daily budget is enforced separately; 30 min is
        // conservative enough to stay well under the search quota.
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

        // Composio's wrapping places GitHub's `{ login, id, name, ... }`
        // user payload at `data` or `data.user`. `pick_str` walks
        // dotted paths so both forms work.
        let data = &resp.data;
        let username = sync::extract_viewer_login(data);
        let display_name = pick_str(data, &["name", "data.name", "user.name", "data.user.name"]);
        let email = pick_str(
            data,
            &["email", "data.email", "user.email", "data.user.email"],
        );
        let avatar_url = pick_str(
            data,
            &[
                "avatar_url",
                "data.avatar_url",
                "user.avatar_url",
                "data.user.avatar_url",
            ],
        );
        let profile_url = pick_str(
            data,
            &[
                "html_url",
                "data.html_url",
                "user.html_url",
                "data.user.html_url",
            ],
        );

        Ok(ProviderUserProfile {
            toolkit: "github".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username,
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

        // ── Step 2: check daily budget ──────────────────────────────
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

        // ── Step 3: resolve the authenticated viewer's login ────────
        //
        // `assignee:<login>` is the GitHub search qualifier. We need
        // the current user's login to scope the sync to "issues
        // assigned to me" — never accept a caller-supplied value here,
        // because that's the privacy boundary.
        let viewer_login = match self.resolve_viewer_login(ctx, &mut state).await {
            Ok(login) => login,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        // Re-check the budget here — `resolve_viewer_login` just spent
        // one request, and if that pushed us over the cap, firing
        // `GITHUB_SEARCH_ISSUES` would be wasted work. Same discipline
        // ClickUp / Linear got after CodeRabbit feedback on the
        // earlier provider PRs.
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:github] budget exhausted after viewer probe, skipping sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "github".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "github sync skipped: daily budget exhausted after viewer probe"
                    .to_string(),
                details: json!({ "budget_exhausted": true, "viewer_login_resolved": true }),
            });
        }

        // ── Step 4: paginated incremental fetch ─────────────────────
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        // GitHub Search query — the `is:issue` qualifier excludes PRs
        // (which share the issues namespace under GitHub's data model
        // but we treat as a separate follow-up surface). `assignee:`
        // scopes to the current user, `sort:updated-desc` makes the
        // freshest items arrive first so the cursor advances quickly.
        let query = format!("is:issue assignee:{viewer_login} sort:updated-desc");

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_updated: Option<String> = None;

        // GitHub Search uses 1-indexed page numbers.
        for page_num in 1..=MAX_PAGES_PER_SYNC {
            if state.budget_exhausted() {
                tracing::info!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:github] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            let args = json!({
                "q": query.clone(),
                "page": page_num,
                "per_page": page_size,
                "sort": "updated",
                "order": "desc"
            });

            // Transport-level failure must persist the request
            // counters / synced markers accumulated earlier in this
            // sync pass — otherwise a flap mid-pagination silently
            // rolls back budget accounting. Same fix CodeRabbit
            // pointed out on the Linear PR (#2402).
            let resp = match ctx.execute(ACTION_SEARCH_ISSUES, Some(args)).await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = state.save(&memory).await;
                    return Err(format!(
                        "[composio:github] {ACTION_SEARCH_ISSUES} page {page_num}: {e:#}"
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
                    "[composio:github] {ACTION_SEARCH_ISSUES} page {page_num}: {err}"
                ));
            }

            let issues = sync::extract_issues(&resp.data);
            total_fetched += issues.len();

            if issues.is_empty() {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:github] empty page, stopping pagination"
                );
                break;
            }

            // ── Per-item dedup + persist ───────────────────────────
            let mut hit_cursor_boundary = false;
            for issue in &issues {
                let Some(issue_id) =
                    crate::openhuman::composio::providers::sync_state::extract_item_id(
                        issue,
                        ISSUE_ID_PATHS,
                    )
                else {
                    tracing::debug!(
                        connection_id = %connection_id,
                        "[composio:github] issue missing id, skipping"
                    );
                    continue;
                };

                let updated = sync::extract_issue_updated(issue);

                // Track newest `updated_at` for cursor advancement.
                if let Some(ref ts) = updated {
                    if newest_updated.as_ref().is_none_or(|existing| ts > existing) {
                        newest_updated = Some(ts.clone());
                    }
                }

                // Composite (issue_id, updated_at) sync key — re-syncs
                // edited issues. Same trick as Notion's
                // `last_edited_time`, ClickUp's `date_updated`, and
                // Linear's `updatedAt`.
                let sync_key = match &updated {
                    Some(ts) => format!("{issue_id}@{ts}"),
                    None => issue_id.clone(),
                };

                // If `updated_at` is at or older than our cursor *and*
                // we've already synced this composite key, the rest
                // of the page is by definition older too — stop early.
                if let (Some(ref cursor), Some(ref ts)) = (&state.cursor, &updated) {
                    if ts <= cursor && state.is_synced(&sync_key) {
                        hit_cursor_boundary = true;
                        continue;
                    }
                }

                if state.is_synced(&sync_key) {
                    continue;
                }

                let title_text = sync::extract_issue_title(issue)
                    .unwrap_or_else(|| format!("GitHub issue {issue_id}"));
                let identifier_hint = sync::extract_repo_qualified_identifier(issue);
                let doc_id = format!("composio-github-issue-{issue_id}");
                // Surface the canonical `owner/repo#number` form in
                // the title when available — that's how humans refer
                // to GitHub issues in conversation. Fall back to a
                // generic prefix when the repo path is missing.
                let title = match identifier_hint {
                    Some(ident) => format!("GitHub {ident}: {title_text}"),
                    None => format!("GitHub: {title_text}"),
                };

                match persist_single_item(
                    &memory,
                    "github",
                    &doc_id,
                    &title,
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
                            connection_id = %connection_id,
                            issue_id = %issue_id,
                            error = %e,
                            "[composio:github] failed to persist issue (continuing)"
                        );
                    }
                }
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:github] reached cursor boundary, stopping pagination"
                );
                break;
            }

            // GitHub Search signals end-of-results implicitly: when
            // fewer than `per_page` results come back, there are no
            // more pages. (The total cap of 1000 is also enforced
            // server-side; once we exceed 10 pages of 100 the search
            // returns an empty list.)
            if (issues.len() as u32) < page_size {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    returned = issues.len(),
                    "[composio:github] short page, end of results"
                );
                break;
            }
        }

        // ── Step 5: advance cursor and save state ───────────────────
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
                "viewer_login": viewer_login,
            }),
        })
    }
}

impl GitHubProvider {
    /// Look up (and budget-record) the authenticated viewer's GitHub
    /// login. Stable for the connection's lifetime but re-fetched on
    /// every sync because the call is cheap and re-fetching
    /// implicitly validates the OAuth connection is still good before
    /// we start paginating.
    async fn resolve_viewer_login(
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

        sync::extract_viewer_login(&resp.data).ok_or_else(|| {
            "[composio:github] GITHUB_GET_AUTHENTICATED_USER returned no login".to_string()
        })
    }
}
