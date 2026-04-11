//! Gmail provider — native Rust counterpart to the QuickJS gmail skill.
//!
//! Mirrors the high-level shape of the JS skill in
//! `tinyhumansai/openhuman-skills/skills/gmail/index.js`:
//!
//!   * On connection / periodic tick → fetch the user profile
//!     (`GMAIL_GET_PROFILE`) and a window of recent message metadata
//!     (`GMAIL_FETCH_EMAILS`).
//!   * Persist a JSON snapshot of the result into the global memory
//!     layer under namespace `composio-gmail` so the agent loop can
//!     surface it via `recall_memory`.
//!   * On `GMAIL_NEW_GMAIL_MESSAGE` triggers → run an incremental
//!     sync so newly arrived mail makes it into memory promptly.
//!
//! All upstream API access goes through
//! [`super::ProviderContext::client`] which proxies to the openhuman
//! backend's `/agent-integrations/composio/execute` endpoint. This
//! provider never holds raw OAuth tokens or hits Composio directly.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{
    pick_str, ComposioProvider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};

/// Composio action slugs used by this provider. Hoisted to constants so
/// they're easy to grep + adjust if Composio renames them upstream.
const ACTION_GET_PROFILE: &str = "GMAIL_GET_PROFILE";
const ACTION_FETCH_EMAILS: &str = "GMAIL_FETCH_EMAILS";

/// Default page size for the periodic email pull. Kept conservative —
/// the goal is "freshness for the agent", not a full archive backfill.
const FETCH_EMAILS_LIMIT: u32 = 25;

/// Memory namespace prefix used when persisting sync snapshots. Mirrors
/// the `skill-{id}` convention in [`crate::openhuman::memory::store::client`]
/// so namespace listings stay coherent across composio + js skills.
const MEMORY_NAMESPACE: &str = "composio-gmail";

pub struct GmailProvider;

impl GmailProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GmailProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for GmailProvider {
    fn toolkit_slug(&self) -> &'static str {
        "gmail"
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 15 minutes — matches the default `syncIntervalMinutes` the
        // QuickJS gmail skill uses.
        Some(15 * 60)
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:gmail] fetch_user_profile via {ACTION_GET_PROFILE}"
        );

        let resp = ctx
            .client
            .execute_tool(ACTION_GET_PROFILE, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:gmail] {ACTION_GET_PROFILE} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:gmail] {ACTION_GET_PROFILE}: {err}"));
        }

        let data = &resp.data;
        // Composio wraps results in `{ data: { ... }, successful: bool }`
        // and the upstream Gmail API returns `{ emailAddress, messagesTotal,
        // threadsTotal, historyId }`. We dig through both `data` and the
        // raw root because backend wrappers occasionally collapse the
        // outer envelope.
        let email = pick_str(
            data,
            &[
                "data.emailAddress",
                "data.email",
                "emailAddress",
                "email",
                "data.profile.emailAddress",
            ],
        );
        let display_name = pick_str(
            data,
            &[
                "data.name",
                "data.profile.name",
                "name",
                "displayName",
                "data.displayName",
            ],
        )
        .or_else(|| email.clone());

        let profile = ProviderUserProfile {
            toolkit: "gmail".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username: None,
            avatar_url: None,
            extras: data.clone(),
        };
        tracing::info!(
            connection_id = ?profile.connection_id,
            email = ?profile.email,
            "[composio:gmail] fetched user profile"
        );
        Ok(profile)
    }

    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        let started_at_ms = now_ms();
        tracing::info!(
            connection_id = ?ctx.connection_id,
            reason = reason.as_str(),
            "[composio:gmail] sync starting"
        );

        // For initial syncs, we ask for a slightly larger window so the
        // first impression of the user's inbox is meaningful. Periodic
        // ticks stay small.
        let limit = match reason {
            SyncReason::ConnectionCreated => FETCH_EMAILS_LIMIT * 2,
            _ => FETCH_EMAILS_LIMIT,
        };
        let args = json!({
            "max_results": limit,
            "query": "in:inbox -in:spam -in:trash",
        });

        let resp = ctx
            .client
            .execute_tool(ACTION_FETCH_EMAILS, Some(args))
            .await
            .map_err(|e| format!("[composio:gmail] {ACTION_FETCH_EMAILS} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:gmail] {ACTION_FETCH_EMAILS}: {err}"));
        }

        let messages = extract_messages(&resp.data);
        let items_ingested = persist_messages(ctx, &messages).await;
        let finished_at_ms = now_ms();

        let summary = format!(
            "gmail sync ({reason}): fetched {fetched} message(s), persisted {persisted}",
            reason = reason.as_str(),
            fetched = messages.len(),
            persisted = items_ingested,
        );
        tracing::info!(
            connection_id = ?ctx.connection_id,
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            fetched = messages.len(),
            persisted = items_ingested,
            "[composio:gmail] sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "gmail".to_string(),
            connection_id: ctx.connection_id.clone(),
            reason: reason.as_str().to_string(),
            items_ingested,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "messages_fetched": messages.len(),
                "limit": limit,
            }),
        })
    }

    async fn on_trigger(
        &self,
        ctx: &ProviderContext,
        trigger: &str,
        _payload: &Value,
    ) -> Result<(), String> {
        tracing::info!(
            connection_id = ?ctx.connection_id,
            trigger = %trigger,
            "[composio:gmail] on_trigger"
        );

        // Only react to message-arrival triggers — other gmail triggers
        // (label changes, etc.) don't justify a full sync round-trip.
        if trigger.eq_ignore_ascii_case("GMAIL_NEW_GMAIL_MESSAGE")
            || trigger.eq_ignore_ascii_case("GMAIL_NEW_MESSAGE")
        {
            // Best-effort incremental pull. Errors here are logged but
            // not propagated — the trigger subscriber doesn't have a
            // user-facing error surface to forward into.
            if let Err(e) = self.sync(ctx, SyncReason::Manual).await {
                tracing::warn!(
                    error = %e,
                    "[composio:gmail] trigger-driven sync failed (non-fatal)"
                );
            }
        }
        Ok(())
    }
}

// ── helpers ────────────────────────────────────────────────────────

/// Walk the Composio response envelope and pull out a list of message
/// objects. Composio is inconsistent about whether the array lives at
/// `data.messages`, `messages`, or `data.data.messages`, so we try a
/// handful of common shapes before giving up.
fn extract_messages(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/messages"),
        data.pointer("/messages"),
        data.pointer("/data/data/messages"),
        data.pointer("/data/items"),
        data.pointer("/items"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr.clone();
        }
    }
    Vec::new()
}

/// Persist a sync snapshot into the global memory store under the
/// `composio-gmail` namespace. Returns the number of items recorded
/// (currently always one document — the snapshot, not per-message
/// rows). Per-message ingestion can come later if/when we add an
/// agent surface that benefits from it.
async fn persist_messages(ctx: &ProviderContext, messages: &[Value]) -> usize {
    let Some(client) = ctx.memory_client() else {
        tracing::debug!("[composio:gmail] memory client not ready, skipping persist");
        return 0;
    };
    if messages.is_empty() {
        return 0;
    }

    let connection_label = ctx
        .connection_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let title = format!("gmail sync — {connection_label}");
    let snapshot = json!({
        "toolkit": "gmail",
        "connection_id": ctx.connection_id,
        "messages": messages,
        "synced_at_ms": now_ms(),
    });
    let content = serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".to_string());

    if let Err(e) = client
        .store_skill_sync(
            // The store_skill_sync helper namespaces as `skill-{id}`,
            // so we pass `gmail` here and rely on the standard prefix.
            // The composio domain reads from `skill-gmail` namespaces
            // through the same memory store as the JS gmail skill —
            // intentional, so the agent's `recall_memory` sees both.
            MEMORY_NAMESPACE.trim_start_matches("composio-"),
            &connection_label,
            &title,
            &content,
            Some("composio-sync".to_string()),
            Some(json!({
                "toolkit": "gmail",
                "connection_id": ctx.connection_id,
                "source": "composio-provider",
            })),
            Some("medium".to_string()),
            None,
            None,
            Some(format!("composio-gmail-{connection_label}")),
        )
        .await
    {
        tracing::warn!(
            error = %e,
            "[composio:gmail] persist snapshot failed (non-fatal)"
        );
        return 0;
    }
    1
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_messages_finds_data_messages() {
        let v = json!({
            "data": { "messages": [{"id": "m1"}, {"id": "m2"}] },
            "successful": true,
        });
        assert_eq!(extract_messages(&v).len(), 2);
    }

    #[test]
    fn extract_messages_finds_top_level_messages() {
        let v = json!({ "messages": [{"id": "m1"}] });
        assert_eq!(extract_messages(&v).len(), 1);
    }

    #[test]
    fn extract_messages_returns_empty_when_missing() {
        let v = json!({ "data": { "other": [] } });
        assert_eq!(extract_messages(&v).len(), 0);
    }

    #[test]
    fn provider_metadata_is_stable() {
        let p = GmailProvider::new();
        assert_eq!(p.toolkit_slug(), "gmail");
        assert_eq!(p.sync_interval_secs(), Some(15 * 60));
    }
}
