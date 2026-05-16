//! Persistent sync state for Composio providers.
//!
//! Each `(toolkit, connection_id)` pair gets its own [`SyncState`] persisted
//! in the local KV store. The state tracks:
//!
//!   * **Cursor** — a provider-specific watermark (e.g. a timestamp or page
//!     token) so the next sync can skip items already seen.
//!   * **Synced IDs** — a set of item identifiers that have been written to
//!     memory. Items in this set are skipped even if they appear again in
//!     an API response (deduplication).
//!   * **Daily request budget** — a rolling counter keyed by calendar date
//!     (`YYYY-MM-DD`) that caps the number of `execute_tool` calls a
//!     provider makes per day. Resets automatically when the date rolls
//!     over.
//!
//! All persistence goes through [`crate::openhuman::memory::MemoryClient`]'s
//! KV surface (`kv_set` / `kv_get` under a dedicated namespace), so the
//! state survives process restarts without any extra file management.

use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::openhuman::memory::MemoryClientRef;

/// Maximum API requests a single provider connection may make per calendar
/// day. This covers the initial backfill case where there are thousands of
/// unsynced items — after this many requests the provider yields and
/// continues on the next day.
pub const DEFAULT_DAILY_REQUEST_LIMIT: u32 = 500;

/// KV namespace under which all sync state keys live. Separate from the
/// memory document namespaces (`skill-gmail`, etc.) to avoid collisions.
pub const KV_NAMESPACE: &str = "composio-sync-state";

/// Persistent sync state for one `(toolkit, connection_id)` pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    /// Toolkit slug, e.g. `"gmail"`.
    pub toolkit: String,
    /// Connection id, e.g. `"conn_abc123"`.
    pub connection_id: String,

    /// Provider-specific cursor. For Gmail this is the internal-date
    /// (epoch millis) of the newest synced message; for Notion it is the
    /// `last_edited_time` ISO string of the most recently synced page.
    /// `None` means "never synced — start from scratch".
    #[serde(default)]
    pub cursor: Option<String>,

    /// Set of item IDs that have already been persisted to memory.
    /// Used for deduplication: if an item appears in an API response
    /// but its ID is in this set, skip it.
    #[serde(default)]
    pub synced_ids: HashSet<String>,

    /// Rolling daily request budget.
    #[serde(default)]
    pub daily_budget: DailyBudget,

    /// ID of the most recently synced item, used by providers (Gmail
    /// today) to short-circuit a tick when the freshest server-side
    /// item matches what we already have. Cheaper than re-walking the
    /// `synced_ids` set — and crucially, it lets us bail out of
    /// pagination on the very first page when nothing has changed,
    /// instead of fetching `MAX_PAGES_PER_SYNC` worth of duplicates
    /// before falling through the per-page dedup loop.
    ///
    /// `None` either when the state is fresh or when an older state
    /// blob was loaded from disk that pre-dates this field.
    #[serde(default)]
    pub last_seen_id: Option<String>,

    /// Unix milliseconds of the last successful sync that wrote into
    /// memory. Lets the adaptive page-cap logic distinguish a "we
    /// synced 30 seconds ago" tick (cap pages aggressively) from a
    /// "we last synced two hours ago" tick (let pagination run to the
    /// usual ceiling). Independent of the periodic scheduler's
    /// in-process `LAST_SYNC_AT` map because that map is rebuilt on
    /// every process restart whereas this value survives restarts.
    ///
    /// `None` until the first successful sync.
    #[serde(default)]
    pub last_sync_at_ms: Option<u64>,
}

/// Tracks the number of API requests made on a given calendar day.
/// Automatically resets when the date rolls over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyBudget {
    /// Calendar date in `YYYY-MM-DD` format.
    pub date: String,
    /// Number of `execute_tool` requests made so far today.
    pub requests_used: u32,
    /// Maximum requests allowed per day.
    pub limit: u32,
}

impl Default for DailyBudget {
    fn default() -> Self {
        Self {
            date: today_str(),
            requests_used: 0,
            limit: DEFAULT_DAILY_REQUEST_LIMIT,
        }
    }
}

impl DailyBudget {
    /// Remaining requests available today. If the stored date is stale
    /// (a previous day), this returns the full limit because the budget
    /// will be reset on the next [`Self::record_request`] call.
    pub fn remaining(&self) -> u32 {
        if self.date != today_str() {
            return self.limit;
        }
        self.limit.saturating_sub(self.requests_used)
    }

    /// Returns `true` if the daily budget is exhausted for today.
    pub fn is_exhausted(&self) -> bool {
        self.remaining() == 0
    }

    /// Record `n` API requests. If the date has rolled over, resets the
    /// counter before adding.
    pub fn record_requests(&mut self, n: u32) {
        let today = today_str();
        if self.date != today {
            self.date = today;
            self.requests_used = 0;
        }
        self.requests_used = self.requests_used.saturating_add(n);
    }

    /// Record a single API request.
    pub fn record_request(&mut self) {
        self.record_requests(1);
    }
}

impl SyncState {
    /// Create a fresh state for a new connection (never synced).
    pub fn new(toolkit: impl Into<String>, connection_id: impl Into<String>) -> Self {
        Self {
            toolkit: toolkit.into(),
            connection_id: connection_id.into(),
            cursor: None,
            synced_ids: HashSet::new(),
            daily_budget: DailyBudget::default(),
            last_seen_id: None,
            last_sync_at_ms: None,
        }
    }

    /// Record the freshest item id observed on a successful sync.
    /// Idempotent — repeated calls with the same id are no-ops.
    pub fn set_last_seen_id(&mut self, item_id: impl Into<String>) {
        self.last_seen_id = Some(item_id.into());
    }

    /// Record the wall-clock time of a successful sync (unix
    /// milliseconds). Persisted alongside the cursor so the adaptive
    /// page-cap survives process restarts.
    pub fn set_last_sync_at_ms(&mut self, ms: u64) {
        self.last_sync_at_ms = Some(ms);
    }

    /// Whether the daily request budget is exhausted.
    pub fn budget_exhausted(&self) -> bool {
        self.daily_budget.is_exhausted()
    }

    /// Remaining API requests for today.
    pub fn budget_remaining(&self) -> u32 {
        self.daily_budget.remaining()
    }

    /// Record API requests made.
    pub fn record_requests(&mut self, n: u32) {
        self.daily_budget.record_requests(n);
    }

    /// Check if an item ID has already been synced.
    pub fn is_synced(&self, item_id: &str) -> bool {
        self.synced_ids.contains(item_id)
    }

    /// Mark an item ID as synced.
    pub fn mark_synced(&mut self, item_id: impl Into<String>) {
        self.synced_ids.insert(item_id.into());
    }

    /// Update the cursor to a new watermark value.
    pub fn advance_cursor(&mut self, cursor: impl Into<String>) {
        self.cursor = Some(cursor.into());
    }

    /// KV key for this state. Deterministic so load + save are symmetric.
    fn kv_key(&self) -> String {
        format!("{}:{}", self.toolkit, self.connection_id)
    }

    /// Load sync state from the KV store, or return a fresh default if
    /// none exists.
    pub async fn load(
        memory: &MemoryClientRef,
        toolkit: &str,
        connection_id: &str,
    ) -> Result<Self, String> {
        let key = format!("{toolkit}:{connection_id}");
        match memory.kv_get(Some(KV_NAMESPACE), &key).await? {
            Some(value) => {
                let mut state: SyncState = serde_json::from_value(value)
                    .map_err(|e| format!("[sync_state] deserialize failed for {key}: {e}"))?;
                // Ensure budget rolls over if date changed.
                if state.daily_budget.date != today_str() {
                    tracing::debug!(
                        toolkit,
                        connection_id,
                        old_date = %state.daily_budget.date,
                        "[sync_state] daily budget rolled over"
                    );
                    state.daily_budget.date = today_str();
                    state.daily_budget.requests_used = 0;
                }
                tracing::debug!(
                    toolkit,
                    connection_id,
                    cursor = ?state.cursor,
                    synced_ids_count = state.synced_ids.len(),
                    budget_remaining = state.budget_remaining(),
                    "[sync_state] loaded"
                );
                Ok(state)
            }
            None => {
                tracing::debug!(
                    toolkit,
                    connection_id,
                    "[sync_state] no existing state, starting fresh"
                );
                Ok(Self::new(toolkit, connection_id))
            }
        }
    }

    /// Persist the current state to the KV store.
    pub async fn save(&self, memory: &MemoryClientRef) -> Result<(), String> {
        let key = self.kv_key();
        let value = serde_json::to_value(self)
            .map_err(|e| format!("[sync_state] serialize failed: {e}"))?;
        memory.kv_set(Some(KV_NAMESPACE), &key, &value).await?;
        tracing::debug!(
            toolkit = %self.toolkit,
            connection_id = %self.connection_id,
            cursor = ?self.cursor,
            synced_ids_count = self.synced_ids.len(),
            budget_used = self.daily_budget.requests_used,
            "[sync_state] saved"
        );
        Ok(())
    }
}

/// Today's date as `YYYY-MM-DD` in UTC.
fn today_str() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Extract an ID string from a JSON value, trying multiple candidate paths.
/// Returns the first non-empty string found.
pub fn extract_item_id(item: &serde_json::Value, paths: &[&str]) -> Option<String> {
    for path in paths {
        let mut cur = item;
        let mut ok = true;
        for segment in path.split('.') {
            match cur.get(segment) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if let Some(s) = cur.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Helper to persist a single item as its own memory document.
///
/// Each item is stored under the provider's memory namespace with a
/// deterministic `document_id` so repeated syncs upsert rather than
/// duplicate. Returns the document ID on success.
pub async fn persist_single_item(
    memory: &MemoryClientRef,
    namespace_skill_id: &str,
    document_id: &str,
    title: &str,
    item: &serde_json::Value,
    toolkit: &str,
    connection_id: Option<&str>,
) -> Result<String, String> {
    let content = serde_json::to_string_pretty(item).unwrap_or_else(|_| "{}".to_string());
    memory
        .store_skill_sync(
            namespace_skill_id,
            connection_id.unwrap_or("default"),
            title,
            &content,
            Some("composio-sync".to_string()),
            Some(json!({
                "toolkit": toolkit,
                "connection_id": connection_id,
                "source": "composio-provider-incremental",
            })),
            Some("medium".to_string()),
            None,
            None,
            Some(document_id.to_string()),
        )
        .await?;
    Ok(document_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_budget_defaults_to_full() {
        let b = DailyBudget::default();
        assert_eq!(b.remaining(), DEFAULT_DAILY_REQUEST_LIMIT);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn daily_budget_tracks_requests() {
        let mut b = DailyBudget::default();
        b.record_requests(100);
        assert_eq!(b.remaining(), DEFAULT_DAILY_REQUEST_LIMIT - 100);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn daily_budget_exhaustion() {
        let mut b = DailyBudget::default();
        b.record_requests(DEFAULT_DAILY_REQUEST_LIMIT);
        assert_eq!(b.remaining(), 0);
        assert!(b.is_exhausted());
    }

    #[test]
    fn daily_budget_saturates_on_overflow() {
        let mut b = DailyBudget::default();
        b.record_requests(DEFAULT_DAILY_REQUEST_LIMIT + 100);
        assert_eq!(b.remaining(), 0);
    }

    #[test]
    fn daily_budget_resets_on_date_change() {
        let mut b = DailyBudget {
            date: "2025-01-01".to_string(),
            requests_used: 499,
            limit: DEFAULT_DAILY_REQUEST_LIMIT,
        };
        // Calling remaining() when date is stale returns full limit.
        assert_eq!(b.remaining(), DEFAULT_DAILY_REQUEST_LIMIT);
        // Recording a request resets the counter.
        b.record_request();
        assert_eq!(b.date, today_str());
        assert_eq!(b.requests_used, 1);
    }

    #[test]
    fn sync_state_deduplication() {
        let mut state = SyncState::new("gmail", "conn_1");
        assert!(!state.is_synced("msg_abc"));
        state.mark_synced("msg_abc");
        assert!(state.is_synced("msg_abc"));
        assert!(!state.is_synced("msg_xyz"));
    }

    #[test]
    fn sync_state_cursor_advancement() {
        let mut state = SyncState::new("notion", "conn_2");
        assert!(state.cursor.is_none());
        state.advance_cursor("2026-04-01T00:00:00Z");
        assert_eq!(state.cursor.as_deref(), Some("2026-04-01T00:00:00Z"));
        state.advance_cursor("2026-04-10T00:00:00Z");
        assert_eq!(state.cursor.as_deref(), Some("2026-04-10T00:00:00Z"));
    }

    #[test]
    fn sync_state_serialization_roundtrip() {
        let mut state = SyncState::new("gmail", "conn_test");
        state.advance_cursor("12345");
        state.mark_synced("item_a");
        state.mark_synced("item_b");
        state.daily_budget.record_requests(42);
        state.set_last_seen_id("msg_top");
        state.set_last_sync_at_ms(1_700_000_000_000);

        let json = serde_json::to_value(&state).unwrap();
        let restored: SyncState = serde_json::from_value(json).unwrap();

        assert_eq!(restored.toolkit, "gmail");
        assert_eq!(restored.connection_id, "conn_test");
        assert_eq!(restored.cursor.as_deref(), Some("12345"));
        assert!(restored.synced_ids.contains("item_a"));
        assert!(restored.synced_ids.contains("item_b"));
        assert_eq!(restored.synced_ids.len(), 2);
        assert_eq!(restored.daily_budget.requests_used, 42);
        assert_eq!(restored.last_seen_id.as_deref(), Some("msg_top"));
        assert_eq!(restored.last_sync_at_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn sync_state_deserializes_legacy_blob_without_new_fields() {
        // Older state blobs serialized before #1404 had no
        // `last_seen_id` / `last_sync_at_ms` keys — make sure the
        // deserializer still accepts them so existing users don't
        // lose their cursor + dedup set on first upgrade.
        let legacy = serde_json::json!({
            "toolkit": "gmail",
            "connection_id": "conn_old",
            "cursor": "1699000000000",
            "synced_ids": ["m1", "m2"],
            "daily_budget": { "date": today_str(), "requests_used": 7, "limit": 500 }
        });
        let restored: SyncState = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.cursor.as_deref(), Some("1699000000000"));
        assert_eq!(restored.synced_ids.len(), 2);
        assert!(restored.last_seen_id.is_none());
        assert!(restored.last_sync_at_ms.is_none());
    }

    #[test]
    fn set_last_seen_id_overwrites_previous_value() {
        let mut state = SyncState::new("gmail", "c");
        state.set_last_seen_id("a");
        state.set_last_seen_id("b");
        assert_eq!(state.last_seen_id.as_deref(), Some("b"));
    }

    #[test]
    fn set_last_sync_at_ms_records_value() {
        let mut state = SyncState::new("gmail", "c");
        state.set_last_sync_at_ms(123);
        state.set_last_sync_at_ms(456);
        assert_eq!(state.last_sync_at_ms, Some(456));
    }

    #[test]
    fn extract_item_id_walks_paths() {
        let item = serde_json::json!({
            "id": "top_level",
            "data": { "id": "nested" }
        });
        assert_eq!(
            extract_item_id(&item, &["data.id", "id"]),
            Some("nested".to_string())
        );
        assert_eq!(
            extract_item_id(&item, &["missing", "id"]),
            Some("top_level".to_string())
        );
        assert_eq!(extract_item_id(&item, &["nope"]), None);
    }

    #[test]
    fn kv_key_is_deterministic() {
        let s1 = SyncState::new("gmail", "conn_x");
        let s2 = SyncState::new("gmail", "conn_x");
        assert_eq!(s1.kv_key(), s2.kv_key());
        assert_eq!(s1.kv_key(), "gmail:conn_x");
    }
}
