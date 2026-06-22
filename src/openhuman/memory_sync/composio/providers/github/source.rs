//! GitHub's [`IncrementalSource`] primitives.
//!
//! GitHub rides the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`GitHubProvider::sync`](super::provider::GitHubProvider) delegates to
//! [`run_github_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the precise `max_items` clamp, cursor advance/hold,
//! state persistence); this module supplies only the GitHub-specific shapes.
//!
//! GitHub is **flat but identity-scoped**: [`GitHubSource::preamble`] resolves
//! the authenticated login and returns a single [`SyncScope`] carrying it, then
//! the orchestrator pages straight through
//! `GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` with `involves:{login}` (1-indexed
//! `page`). Per-item dedup is keyed by `{issue_id}@{updated_at}`.
//!
//! **Server-side depth window.** Unlike the other flat providers, GitHub
//! applies `sync_depth_days` server-side by injecting `updated:>{date}` into the
//! search query on the first sync (before a cursor exists) — so it overrides
//! [`IncrementalSource::server_side_depth`] and the orchestrator skips its
//! client-side timestamp truncation. On incremental syncs the persistent cursor
//! (`updated:>{cursor}`) bounds the window instead.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

use super::provider::{build_search_query_with_depth, ACTION_SEARCH_ISSUES};
use super::sync;
use crate::openhuman::memory_sync::composio::providers::orchestrator::{
    self, IncrementalSource, IngestOutcome, PageFetch, SyncItem, SyncScope,
};
use crate::openhuman::memory_sync::composio::providers::sync_state::SyncState;
use crate::openhuman::memory_sync::composio::providers::{
    ProviderContext, SyncOutcome, SyncReason,
};

/// Items per search page on steady-state syncs.
const PAGE_SIZE: u32 = 50;

/// Larger page for the initial post-OAuth backfill.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages per sync pass.
const MAX_PAGES: u32 = 20;

/// GitHub's [`IncrementalSource`].
#[derive(Default)]
pub(crate) struct GitHubSource {
    depth_fragment: Mutex<Option<String>>,
}

/// Entry point used by [`super::provider::GitHubProvider::sync`].
pub(crate) async fn run_github_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    orchestrator::run_sync(&GitHubSource::default(), ctx, reason).await
}

impl GitHubSource {
    /// Resolve the authenticated user's GitHub login. Re-fetched every sync
    /// (rather than cached in `SyncState`) so it implicitly validates the OAuth
    /// token before paginating. Records the request against the budget.
    async fn resolve_login(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:github] resolve_login via {}",
            super::provider::ACTION_GET_AUTHENTICATED_USER
        );

        let resp = ctx
            .execute(
                super::provider::ACTION_GET_AUTHENTICATED_USER,
                Some(json!({})),
            )
            .await
            .map_err(|e| {
                format!(
                    "[composio:github] {} failed: {e:#}",
                    super::provider::ACTION_GET_AUTHENTICATED_USER
                )
            })?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:github] {}: {err}",
                super::provider::ACTION_GET_AUTHENTICATED_USER
            ));
        }

        let login = sync::extract_user_login(&resp.data).ok_or_else(|| {
            "[composio:github] GITHUB_GET_THE_AUTHENTICATED_USER returned no login".to_string()
        })?;

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:github] resolve_login complete"
        );
        Ok(login)
    }

    fn set_depth_fragment(&self, fragment: Option<String>) {
        let mut guard = self
            .depth_fragment
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = fragment;
    }

    fn depth_fragment(&self) -> Option<String> {
        self.depth_fragment
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl IncrementalSource for GitHubSource {
    fn toolkit(&self) -> &'static str {
        "github"
    }

    fn page_size(&self, reason: SyncReason) -> u32 {
        match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        }
    }

    fn max_pages(&self) -> u32 {
        MAX_PAGES
    }

    /// GitHub applies the depth window server-side (see module docs).
    fn server_side_depth(&self) -> bool {
        true
    }

    fn detail_noun(&self) -> &'static str {
        "issues"
    }

    /// Resolve the login and carry it as the single scope's id.
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
        let depth_fragment = if state.cursor.is_none() {
            ctx.sync_depth_days.map(|days| {
                let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
                format!("updated:>{}", floor.format("%Y-%m-%dT%H:%M:%SZ"))
            })
        } else {
            None
        };
        self.set_depth_fragment(depth_fragment);

        let login = self.resolve_login(ctx, state).await?;
        let label = format!("involves:{login}");
        Ok(vec![SyncScope::nested(login, label)])
    }

    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        cursor: Option<&str>,
        reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String> {
        let login = &scope.id;
        // GitHub paginates by 1-indexed `page`; the orchestrator's opaque cursor
        // carries the next page number (`None` = first page).
        let page_num: u32 = cursor.and_then(|c| c.parse().ok()).unwrap_or(1);
        let page_size = self.page_size(reason);

        // Stable for the whole sync pass: GitHub's `page` is relative to the
        // exact query, so the depth floor must not move while paginating.
        let depth_fragment = self.depth_fragment();
        let query = build_search_query_with_depth(
            login,
            state.cursor.as_deref(),
            depth_fragment.as_deref(),
        );

        let args = json!({
            "q": query,
            "sort": "updated",
            "order": "desc",
            "per_page": page_size,
            "page": page_num,
        });

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            page_num,
            page_size,
            has_depth_fragment = depth_fragment.is_some(),
            "[composio:github] fetch_page via {ACTION_SEARCH_ISSUES}"
        );

        let resp = ctx
            .execute(ACTION_SEARCH_ISSUES, Some(args))
            .await
            .map_err(|e| {
                format!("[composio:github] {ACTION_SEARCH_ISSUES} page={page_num}: {e:#}")
            })?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:github] {ACTION_SEARCH_ISSUES} page={page_num}: {err}"
            ));
        }

        let issues = sync::extract_issues(&resp.data);
        // A short page means the result set is exhausted — no next page.
        let next = if (issues.len() as u32) < page_size {
            None
        } else {
            Some((page_num + 1).to_string())
        };

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            page_num,
            fetched = issues.len(),
            has_next = next.is_some(),
            "[composio:github] fetch_page complete"
        );

        Ok(PageFetch {
            items: issues,
            next,
        })
    }

    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        let issue_id = sync::extract_issue_id(item)?;
        match sync::extract_issue_updated_at(item) {
            Some(updated) => Some(format!("{issue_id}@{updated}")),
            None => Some(issue_id),
        }
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        sync::extract_issue_updated_at(item)
    }

    /// GitHub ingests sequentially (parity with the prior per-item loop) — one
    /// issue at a time into the memory-tree pipeline.
    async fn ingest(
        &self,
        ctx: &ProviderContext,
        _scope: &SyncScope,
        _state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");
        let mut outcome = IngestOutcome::default();
        for it in items {
            let Some(issue_id) = sync::extract_issue_id(&it.raw) else {
                continue;
            };
            let title = sync::extract_issue_title(&it.raw)
                .unwrap_or_else(|| format!("GitHub issue {issue_id}"));
            match super::ingest::ingest_issue_into_memory_tree(
                &ctx.config,
                connection_id,
                &issue_id,
                &title,
                it.sort_ts.as_deref(),
                &it.raw,
            )
            .await
            {
                Ok(_chunks_written) => {
                    outcome.synced_keys.push(it.dedup_key);
                    outcome.persisted += 1;
                }
                Err(e) => {
                    outcome.had_failures = true;
                    tracing::warn!(
                        issue_id = %issue_id,
                        error = %e,
                        "[composio:github] failed to ingest issue into memory_tree (continuing)"
                    );
                }
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn item_dedup_key_composes_id_and_updated() {
        let with_update = json!({ "id": 42, "number": 7, "updated_at": "2026-05-01T00:00:00Z" });
        // GitHub ids may be numeric; extract_issue_id renders them as strings.
        let key = GitHubSource::default().item_dedup_key(&with_update);
        assert!(
            key.as_deref().map(|k| k.contains('@')).unwrap_or(false),
            "expected composite id@updated key, got {key:?}"
        );
    }
}
