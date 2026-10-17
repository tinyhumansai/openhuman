//! Notifications on the `tinystoragedrivers` document port.
//!
//! Kept in the storage backend ([`crate::storage`]); the functions in
//! `store.rs` open it per call, and the old `notifications/notifications.db`
//! is read once by the importer ([`import_source`]). Records live under the current call's storage scope (the acting
//! agent, `local` on a single-user host).
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `notifications.integration` | notification id | one ingested notification |
//! | `notifications.dedup` | hash of provider, account, title, body | when that content last arrived |
//! | `notifications.settings` | provider | one provider's settings |
//! | `notifications.core` | `Key::of([workspace, event id])` | one persisted core notification event |
//!
//! The shapes and the v1 -> v2 steps are in [`super::storage_schema`].
//!
//! Optional columns are left out of a document rather than stored as `null`,
//! so "is unscored" is a missing field on every driver. `received_ms` (epoch
//! milliseconds) orders the list, since RFC 3339 strings with differing
//! fractional digits do not sort as instants.
//!
//! The SQL store makes "insert unless the same content arrived in the last
//! minute" atomic with `BEGIN IMMEDIATE`. Here every insert advances a
//! `notification_dedup` document under compare-and-swap first, so two
//! processes ingesting the same notification at once insert it once.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Filter, Precondition, Query, Sort, Versioned};

use super::types::{
    CoreNotificationEvent, IntegrationNotification, NotificationSettings, NotificationStats,
    NotificationStatus,
};
use crate::config::Config;
use crate::storage::documents::{compare_and_swap, text, Repo};
use crate::storage::{DocumentStore, DocumentStoreExt, StorageError};

pub(super) use super::storage_schema::{core_id, CORE, DEDUP, NOTIFICATIONS, SETTINGS};
use super::storage_schema::{legacy_collections, specs};
use crate::storage::schema_support::key_of;

/// The `workspace` key of core notifications in the per-workspace default
/// file (see [`current`]).
pub(super) const LOCAL_WORKSPACE: &str = "local";
const DOMAIN: &str = "notifications::store";

/// How long identical content counts as a duplicate.
const DEDUP_WINDOW_SECS: i64 = 60;

/// The legacy `notifications.db`, as the storage importer reads it.
///
/// Core notifications are keyed by workspace. The legacy table and the old
/// per-file default both used the fixed [`LOCAL_WORKSPACE`] key; a backend
/// that may serve several workspaces gets them re-filed under the real
/// workspace's key, exactly as [`current`] does for new records
/// ([`workspace_key_for`]).
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "notifications",
        owner: "notifications",
        db: "notifications/notifications.db",
        collections: legacy_collections,
        read_legacy: |scratch, real| {
            super::store_import::read_for(scratch, &workspace_key_for(real))
        },
        rekey: |mut record, config| {
            if record.collection != super::storage_schema::LEGACY_CORE {
                return record;
            }
            let prefix = super::store_import::legacy_core_id(LOCAL_WORKSPACE, "");
            let Some(event_id) = record.id.strip_prefix(&prefix) else {
                return record;
            };
            let workspace = workspace_key_for(config);
            record.id = super::store_import::legacy_core_id(&workspace, event_id);
            if let Some(doc) = record.doc.as_object_mut() {
                doc.insert("workspace".into(), serde_json::json!(workspace));
            }
            record
        },
    }
}

fn collections() -> Vec<CollectionSpec> {
    specs()
}

fn workspace_key(path: &std::path::Path) -> String {
    format!("path:{}", hex::encode(path.as_os_str().as_encoded_bytes()))
}

/// The `workspace` key core notifications are filed under for `config`.
///
/// A backend that is the workspace's own default (`sqlite:<workspace>/storage`,
/// or the one opened lazily when none is installed) lives in the workspace and
/// moves with it, so it uses a fixed key: a renamed or restored workspace
/// still finds its notifications. Any other backend may serve several
/// workspaces, so it keeps them apart by workspace path.
pub(super) fn workspace_key_for(config: &Config) -> String {
    let own_default = crate::storage::config::default_url(&config.workspace_dir);
    match crate::storage::installed_url() {
        Some(url) if url != own_default => workspace_key(&config.workspace_dir),
        _ => LOCAL_WORKSPACE.to_string(),
    }
}

/// The document store for this call: the storage backend, in the acting scope.
pub(super) fn current(config: &Config) -> Result<Docs> {
    let repo = Repo::current_for(&config.workspace_dir, DOMAIN, collections)?;
    Ok(Docs(repo, workspace_key_for(config)))
}

fn status_of(raw: Option<&str>) -> NotificationStatus {
    match raw {
        Some("read") => NotificationStatus::Read,
        Some("acted") => NotificationStatus::Acted,
        Some("dismissed") => NotificationStatus::Dismissed,
        _ => NotificationStatus::Unread,
    }
}

fn from_ms(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::from_timestamp_millis)
}

pub(super) fn to_doc(n: &IntegrationNotification) -> Value {
    let mut doc = Map::new();
    doc.insert("provider".into(), json!(n.provider));
    doc.insert("title".into(), json!(n.title));
    doc.insert("body".into(), json!(n.body));
    doc.insert("raw_payload".into(), json!(n.raw_payload.to_string()));
    doc.insert("status".into(), json!(n.status.as_str()));
    doc.insert(
        "received_ms".into(),
        json!(n.received_at.timestamp_millis()),
    );
    let optional = [
        ("account_id", n.account_id.as_ref().map(|v| json!(v))),
        ("importance_score", n.importance_score.map(|v| json!(v))),
        ("triage_action", n.triage_action.as_ref().map(|v| json!(v))),
        ("triage_reason", n.triage_reason.as_ref().map(|v| json!(v))),
        (
            "scored_ms",
            n.scored_at.map(|v| json!(v.timestamp_millis())),
        ),
    ];
    for (field, value) in optional {
        if let Some(value) = value {
            doc.insert(field.into(), value);
        }
    }
    Value::Object(doc)
}

/// The notification id a document id was built from (`Key::of` of it).
fn notification_id_of(id: &str) -> String {
    tinystoragedrivers::Key::new(id)
        .ok()
        .and_then(|key| key.split())
        .and_then(|mut parts| (parts.len() == 1).then(|| parts.remove(0)))
        .unwrap_or_else(|| id.to_string())
}

fn to_notification(stored: &Versioned<Value>) -> IntegrationNotification {
    let doc = &stored.doc;
    let raw_payload = text(doc, "raw_payload").map_or(Value::Null, |raw| {
        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
    });
    IntegrationNotification {
        id: notification_id_of(&stored.id),
        provider: text(doc, "provider").unwrap_or_default().to_string(),
        account_id: text(doc, "account_id").map(str::to_string),
        title: text(doc, "title").unwrap_or_default().to_string(),
        body: text(doc, "body").unwrap_or_default().to_string(),
        raw_payload,
        importance_score: doc
            .get("importance_score")
            .and_then(Value::as_f64)
            .map(|score| score as f32),
        triage_action: text(doc, "triage_action").map(str::to_string),
        triage_reason: text(doc, "triage_reason").map(str::to_string),
        status: status_of(text(doc, "status")),
        received_at: from_ms(doc.get("received_ms").and_then(Value::as_i64))
            .unwrap_or_else(Utc::now),
        scored_at: from_ms(doc.get("scored_ms").and_then(Value::as_i64)),
    }
}

/// The dedup document id for one piece of content. Each part is length
/// prefixed, so no two distinct tuples hash the same input.
pub(super) fn dedup_id(
    provider: &str,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> String {
    let mut hasher = Sha256::new();
    for part in [Some(provider), account_id, Some(title), Some(body)] {
        match part {
            Some(part) => hasher.update(format!("{}:{part};", part.len())),
            None => hasher.update("-;"),
        }
    }
    hex::encode(hasher.finalize())
}

/// Records that the content arrived at `now_ms` (the wall clock, never the
/// notification's own timestamp, which a provider controls). With
/// `skip_recent`, does nothing and returns `None` when the same content
/// already arrived within [`DEDUP_WINDOW_SECS`] of `now_ms`. Otherwise
/// returns the previous arrival time (`Some(None)` when there was none), so a
/// failed insert can restore it with [`release_content`].
async fn claim_content(
    docs: &Arc<dyn DocumentStore>,
    id: &str,
    now_ms: i64,
    skip_recent: bool,
) -> Result<Option<Option<i64>>, StorageError> {
    let window_start = now_ms - DEDUP_WINDOW_SECS * 1000;
    let repo = tinystoragedrivers::Repo::<Value>::new(docs, DEDUP)?;
    let mut previous: Option<Option<i64>> = None;
    repo.update(id, |current| {
        let last = current
            .as_ref()
            .and_then(|doc| doc.get("last_ms"))
            .and_then(Value::as_i64);
        if skip_recent && last.is_some_and(|last| last >= window_start) {
            previous = None;
            return Ok(None);
        }
        previous = Some(last);
        Ok(Some(json!({ "last_ms": now_ms })))
    })
    .await?;
    Ok(previous)
}

/// Undoes a [`claim_content`] whose notification was never stored, so a retry
/// is not reported as a duplicate. Best effort: it only restores the claim if
/// it is still the one made at `claimed_ms`; a failure is logged, since the
/// caller is already returning the insert's own error.
async fn release_content(
    docs: &Arc<dyn DocumentStore>,
    id: &str,
    claimed_ms: i64,
    previous: Option<i64>,
) {
    let outcome: Result<(), StorageError> = async {
        let Some(stored) = docs.get(DEDUP, id).await? else {
            return Ok(());
        };
        if stored.doc.get("last_ms").and_then(Value::as_i64) != Some(claimed_ms) {
            return Ok(());
        }
        match previous {
            Some(last) => {
                docs.put(DEDUP, id, json!({ "last_ms": last }), stored.unchanged())
                    .await?;
            }
            None => {
                docs.delete(DEDUP, id, stored.unchanged()).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        tracing::warn!(%error, "[notifications::store] could not release dedup claim");
    }
}

/// The notification store over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo, String);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections), String::new())
    }

    /// The `workspace` key this handle files core notifications under.
    pub(super) fn workspace(&self) -> &str {
        &self.1
    }

    /// Inserts `n`, failing if its id exists. With `skip_recent`, returns
    /// `false` instead when the same content arrived in the last minute.
    pub(super) fn insert(&self, n: &IntegrationNotification, skip_recent: bool) -> Result<bool> {
        let id = n.id.clone();
        let doc = to_doc(n);
        let dedup = dedup_id(&n.provider, n.account_id.as_deref(), &n.title, &n.body);
        let now_ms = Utc::now().timestamp_millis();
        self.0.run(|docs| async move {
            let Some(previous) = claim_content(&docs, &dedup, now_ms, skip_recent).await? else {
                return Ok(false);
            };
            if let Err(error) = docs
                .put(NOTIFICATIONS, &id, doc, Precondition::Absent)
                .await
            {
                release_content(&docs, &dedup, now_ms, previous).await;
                return Err(error);
            }
            Ok(true)
        })
    }

    pub(super) fn exists_recent(
        &self,
        provider: &str,
        account_id: Option<&str>,
        title: &str,
        body: &str,
    ) -> Result<bool> {
        let id = dedup_id(provider, account_id, title, body);
        let window_start = (Utc::now() - Duration::seconds(DEDUP_WINDOW_SECS)).timestamp_millis();
        self.0.run(|docs| async move {
            Ok(docs
                .get(DEDUP, &id)
                .await?
                .and_then(|stored| stored.doc.get("last_ms").and_then(Value::as_i64))
                .is_some_and(|last| last >= window_start))
        })
    }

    pub(super) fn list(
        &self,
        limit: usize,
        offset: usize,
        provider: Option<&str>,
        min_score: Option<f32>,
    ) -> Result<Vec<IntegrationNotification>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let filter = [
            provider.map(|provider| Filter::eq("provider", provider)),
            min_score.map(|score| {
                Filter::exists("importance_score", false)
                    .or(Filter::gte("importance_score", f64::from(score)))
            }),
        ]
        .into_iter()
        .flatten()
        .reduce(Filter::and)
        .unwrap_or(Filter::All);
        let query = Query::filter(filter)
            .sort(Sort::desc("received_ms"))
            .limit(offset.saturating_add(limit));
        self.0.run(|docs| async move {
            let page = docs.query(NOTIFICATIONS, &query).await?;
            Ok(page
                .items
                .iter()
                .skip(offset)
                .take(limit)
                .map(to_notification)
                .collect())
        })
    }

    pub(super) fn update_triage(
        &self,
        id: &str,
        score: f32,
        action: &str,
        reason: &str,
    ) -> Result<bool> {
        let (score, action, reason) = (f64::from(score), action.to_string(), reason.to_string());
        let scored_ms = Utc::now().timestamp_millis();
        self.set(id, move |next| {
            next["importance_score"] = json!(score);
            next["triage_action"] = json!(action);
            next["triage_reason"] = json!(reason);
            next["scored_ms"] = json!(scored_ms);
        })
    }

    pub(super) fn set_status(&self, id: &str, status: NotificationStatus) -> Result<bool> {
        self.set(id, move |next| next["status"] = json!(status.as_str()))
    }

    /// Applies `edit` to notification `id`; `false` when it does not exist.
    fn set(&self, id: &str, edit: impl Fn(&mut Value) + Send + Sync + 'static) -> Result<bool> {
        let id = id.to_string();
        self.0.run(|docs| async move {
            let changed = compare_and_swap(&docs, NOTIFICATIONS, &id, |doc| {
                let mut next = doc.clone();
                edit(&mut next);
                Some(next)
            })
            .await?;
            Ok(changed.is_some())
        })
    }

    pub(super) fn unread_count(&self) -> Result<i64> {
        self.0.run(|docs| async move {
            let count = docs
                .count(NOTIFICATIONS, &Filter::eq("status", "unread"))
                .await?;
            Ok(i64::try_from(count).unwrap_or(i64::MAX))
        })
    }

    /// Aggregates over every notification in the scope. The port has no
    /// `GROUP BY`, so the counts are folded here; the notification center
    /// holds one user's notifications, which keeps this bounded in practice.
    pub(super) fn stats(&self) -> Result<NotificationStats> {
        self.0.run(|docs| async move {
            let mut stats = NotificationStats {
                total: 0,
                unread: 0,
                unscored: 0,
                by_provider: HashMap::new(),
                by_action: HashMap::new(),
            };
            for stored in docs.query_all(NOTIFICATIONS, &Query::all()).await? {
                let doc = &stored.doc;
                stats.total += 1;
                if text(doc, "status") == Some("unread") {
                    stats.unread += 1;
                }
                if doc.get("importance_score").is_none() {
                    stats.unscored += 1;
                }
                let provider = text(doc, "provider").unwrap_or_default().to_string();
                *stats.by_provider.entry(provider).or_default() += 1;
                if let Some(action) = text(doc, "triage_action") {
                    *stats.by_action.entry(action.to_string()).or_default() += 1;
                }
            }
            Ok(stats)
        })
    }

    pub(super) fn upsert_settings(&self, settings: &NotificationSettings) -> Result<()> {
        let id = key_of(&settings.provider)?;
        let doc = json!({
            "enabled": settings.enabled,
            "importance_threshold": f64::from(settings.importance_threshold),
            "route_to_orchestrator": settings.route_to_orchestrator,
        });
        self.0.run(|docs| async move {
            docs.put(SETTINGS, &id, doc, Precondition::None)
                .await
                .map(|_| ())
        })
    }

    pub(super) fn get_settings(&self, provider: &str) -> Result<NotificationSettings> {
        let id = key_of(provider)?;
        let provider = provider.to_string();
        self.0.run(|docs| async move {
            let defaults = NotificationSettings {
                provider: provider.clone(),
                ..NotificationSettings::default()
            };
            let Some(stored) = docs.get(SETTINGS, &id).await? else {
                return Ok(defaults);
            };
            let doc = &stored.doc;
            Ok(NotificationSettings {
                enabled: doc
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(defaults.enabled),
                importance_threshold: doc
                    .get("importance_threshold")
                    .and_then(Value::as_f64)
                    .map_or(defaults.importance_threshold, |value| value as f32),
                route_to_orchestrator: doc
                    .get("route_to_orchestrator")
                    .and_then(Value::as_bool)
                    .unwrap_or(defaults.route_to_orchestrator),
                provider,
            })
        })
    }

    pub(super) fn insert_core_notification(
        &self,
        workspace: &str,
        event: &CoreNotificationEvent,
    ) -> Result<bool> {
        let id = core_id(workspace, &event.id);
        let doc = json!({
            "workspace": workspace,
            "payload": serde_json::to_string(event)
                .context("[notifications::store] serialize core notification failed")?,
            "timestamp_ms": event.timestamp_ms,
            "read": false,
        });
        self.0.run(|docs| async move {
            match docs.put(CORE, &id, doc, Precondition::Absent).await {
                Ok(_) => Ok(true),
                // A re-publish of the same event id is ignored.
                Err(error) if error.kind() == ErrorKind::Conflict => Ok(false),
                Err(error) => Err(error),
            }
        })
    }

    pub(super) fn list_core_notifications(
        &self,
        workspace: &str,
        only_unread: bool,
        limit: usize,
    ) -> Result<Vec<CoreNotificationEvent>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let in_workspace = Filter::eq("workspace", workspace);
        let filter = if only_unread {
            in_workspace.and(Filter::eq("read", false))
        } else {
            in_workspace
        };
        let query = Query::filter(filter)
            .sort(Sort::desc("timestamp_ms"))
            .limit(limit);
        self.0.run(|docs| async move {
            let page = docs.query(CORE, &query).await?;
            Ok(page
                .items
                .iter()
                .filter_map(|stored| {
                    let payload = text(&stored.doc, "payload")?;
                    serde_json::from_str(payload)
                        .map_err(|error| {
                            // A single corrupt record must not break the sync-down.
                            tracing::warn!(
                                %error,
                                "[notifications::store] skipping undeserializable core notification"
                            );
                        })
                        .ok()
                })
                .collect())
        })
    }

    pub(super) fn mark_core_notification_read(&self, workspace: &str, id: &str) -> Result<bool> {
        let id = core_id(workspace, id);
        self.0.run(|docs| async move {
            let marked = compare_and_swap(&docs, CORE, &id, |doc| {
                let mut next = doc.clone();
                next["read"] = json!(true);
                Some(next)
            })
            .await?;
            Ok(marked.is_some())
        })
    }

    pub(super) fn unread_core_notification_count(&self, workspace: &str) -> Result<i64> {
        let filter = Filter::eq("workspace", workspace).and(Filter::eq("read", false));
        self.0.run(|docs| async move {
            let count = docs.count(CORE, &filter).await?;
            Ok(i64::try_from(count).unwrap_or(i64::MAX))
        })
    }
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
