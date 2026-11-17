//! Persistence for `IntegrationNotification` records.
//!
//! Every function here is served from the document port
//! (`store_documents.rs`), in the storage backend and the acting scope. The
//! `notifications/notifications.db` older installs wrote is read once,
//! read-only, by the storage importer (`store_import.rs`).

use anyhow::Result;

use crate::config::Config;

use super::types::{
    CoreNotificationEvent, IntegrationNotification, NotificationSettings, NotificationStatus,
};

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Persist a new notification to the store.
pub fn insert(config: &Config, n: &IntegrationNotification) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.insert(n, false).map(|_| ())
}

// ─────────────────────────────────────────────────────────────────────────────
// Core notification persistence (#3805)
// ─────────────────────────────────────────────────────────────────────────────

/// Persist a core notification event so it survives an app restart / a window
/// where no client was connected to the broadcast bus.
///
/// Idempotent: the event `id` is the primary key, so a re-publish of the same
/// event is ignored (no duplicates). Returns `true` when a new row was written,
/// `false` when an event with the same id already existed.
pub fn insert_core_notification(config: &Config, event: &CoreNotificationEvent) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.insert_core_notification(docs.workspace(), event)
}

/// Return persisted core notifications, newest first. When `only_unread` is
/// true only rows with `read = 0` are returned. Used by the frontend to
/// sync-down notifications missed while the app was closed.
pub fn list_core_notifications(
    config: &Config,
    only_unread: bool,
    limit: usize,
) -> Result<Vec<CoreNotificationEvent>> {
    let docs = super::store_documents::current(config)?;
    docs.list_core_notifications(docs.workspace(), only_unread, limit)
}

/// Mark a persisted core notification as read so it isn't re-surfaced on the
/// next sync-down. Returns `true` when a row was updated.
pub fn mark_core_notification_read(config: &Config, id: &str) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.mark_core_notification_read(docs.workspace(), id)
}

/// Count unread persisted core notifications.
pub fn unread_core_notification_count(config: &Config) -> Result<i64> {
    let docs = super::store_documents::current(config)?;
    docs.unread_core_notification_count(docs.workspace())
}

/// Atomically insert a notification unless a matching one arrived recently.
///
/// Returns `true` when inserted, `false` when skipped as duplicate.
pub fn insert_if_not_recent(config: &Config, n: &IntegrationNotification) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.insert(n, true)
}

/// List notifications with optional filtering.
pub fn list(
    config: &Config,
    limit: usize,
    offset: usize,
    provider_filter: Option<&str>,
    min_score: Option<f32>,
) -> Result<Vec<IntegrationNotification>> {
    let docs = super::store_documents::current(config)?;
    docs.list(limit, offset, provider_filter, min_score)
}

/// Update triage scoring fields in-place.
pub fn update_triage(
    config: &Config,
    id: &str,
    score: f32,
    action: &str,
    reason: &str,
) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.update_triage(id, score, action, reason).map(|found| {
        if !found {
            tracing::warn!(id = %id, action = %action, "[notifications::store] update_triage matched no rows");
        }
    })
}

/// Transition a notification from `unread` to `read`.
pub fn mark_read(config: &Config, id: &str) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.set_status(id, NotificationStatus::Read).map(|found| {
        if !found {
            tracing::warn!(id = %id, "[notifications::store] mark_read matched no rows");
        }
    })
}

/// Count unread notifications.
pub fn unread_count(config: &Config) -> Result<i64> {
    let docs = super::store_documents::current(config)?;
    docs.unread_count()
}

/// Check whether a notification with identical content was received in the
/// last 60 seconds.
pub fn exists_recent(
    config: &Config,
    provider: &str,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.exists_recent(provider, account_id, title, body)
}

/// Transition a notification status to 'dismissed'.
///
/// Returns `true` when at least one row matched and was updated.
pub fn mark_dismissed(config: &Config, id: &str) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.set_status(id, NotificationStatus::Dismissed)
}

/// Transition a notification status to 'acted'.
///
/// Returns `true` when at least one row matched and was updated.
pub fn mark_acted(config: &Config, id: &str) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.set_status(id, NotificationStatus::Acted)
}

/// Return aggregate statistics for the notification intelligence pipeline.
pub fn stats(config: &Config) -> Result<super::types::NotificationStats> {
    let docs = super::store_documents::current(config)?;
    docs.stats()
}

/// Upsert provider-level notification settings.
pub fn upsert_settings(config: &Config, settings: &NotificationSettings) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.upsert_settings(settings)
}

/// Read provider-level notification settings with defaults when missing.
pub fn get_settings(config: &Config, provider: &str) -> Result<NotificationSettings> {
    let docs = super::store_documents::current(config)?;
    docs.get_settings(provider)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;

#[cfg(test)]
#[path = "store_tests_2_tests.rs"]
mod tests;
