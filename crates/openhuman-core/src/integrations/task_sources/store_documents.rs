//! Task sources on the `tinystoragedrivers` document port.
//!
//! Used instead of `task_sources/sources.db` when the host configured a
//! storage backend ([`crate::storage`]); the functions in `store.rs` pick it
//! per call. Records live under the current call's storage scope (the acting
//! agent, `local` on a single-user host).
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `task_sources.sources` | source id | one configured source |
//! | `task_sources.ingested` | `Key::of([source_id, external_id])` | one dedup ledger entry |
//!
//! The shapes and the v1 -> v2 steps are in [`super::storage_schema`].
//!
//! `filter`, `target` and the task `payload` are JSON strings, as in SQLite,
//! so a MongoDB-backed port never meets an arbitrary key. Epoch-millisecond
//! fields (`created_ms`, `ingested_ms`) carry the ordering. Removing a source
//! removes its ledger entries, which the SQL schema did with a cascade.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use tinystoragedrivers::{CollectionSpec, Filter, Precondition, Query, Sort, Versioned};

use super::store::{apply_patch, content_hash, IngestedTaskRef};
use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};
use crate::config::Config;
use crate::integrations::composio::contract::NormalizedTask;
use crate::storage::documents::{compare_and_swap, text, Repo};
use crate::storage::{DocumentStoreExt, StorageError};

pub(super) use super::storage_schema::{ingested_id, INGESTED, SOURCES};
use super::storage_schema::{legacy_collections, specs};
use crate::storage::schema_support::key_of;
const DOMAIN: &str = "task_sources::store";

/// The legacy `sources.db`, as the storage importer reads it.
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "task_sources",
        owner: "task_sources",
        db: "task_sources/sources.db",
        collections: legacy_collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

fn collections() -> Vec<CollectionSpec> {
    specs().into()
}

/// The document store for this call: the storage backend, in the acting scope.
pub(super) fn current(config: &Config) -> Result<Docs> {
    Ok(Docs(Repo::current_for(
        &config.workspace_dir,
        DOMAIN,
        collections,
    )?))
}

pub(super) fn to_doc(source: &TaskSource) -> Result<Value> {
    let mut doc = Map::new();
    doc.insert("provider".into(), json!(source.provider.as_str()));
    doc.insert("enabled".into(), json!(source.enabled));
    doc.insert(
        "filter".into(),
        json!(serde_json::to_string(&source.filter).context("serialize task source filter")?),
    );
    doc.insert("interval_secs".into(), json!(source.interval_secs));
    doc.insert(
        "target".into(),
        json!(serde_json::to_string(&source.target).context("serialize task source target")?),
    );
    doc.insert(
        "max_tasks_per_fetch".into(),
        json!(source.max_tasks_per_fetch),
    );
    doc.insert(
        "created_ms".into(),
        json!(source.created_at.timestamp_millis()),
    );
    let optional = [
        ("connection_id", source.connection_id.clone()),
        ("name", source.name.clone()),
        ("last_status", source.last_status.clone()),
    ];
    for (field, value) in optional {
        if let Some(value) = value {
            doc.insert(field.into(), json!(value));
        }
    }
    if let Some(at) = source.last_fetch_at {
        doc.insert("last_fetch_ms".into(), json!(at.timestamp_millis()));
    }
    Ok(Value::Object(doc))
}

fn from_ms(ms: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
        .ok_or_else(|| anyhow!("timestamp out of range in task source: {ms}"))
}

/// The source id a document id was built from (`Key::of` of it).
fn source_id_of(id: &str) -> String {
    tinystoragedrivers::Key::new(id)
        .ok()
        .and_then(|key| key.split())
        .and_then(|mut parts| (parts.len() == 1).then(|| parts.remove(0)))
        .unwrap_or_else(|| id.to_string())
}

fn to_source(stored: &Versioned<Value>) -> Result<TaskSource> {
    let doc = &stored.doc;
    let field = |name: &str| text(doc, name).ok_or_else(|| anyhow!("task source missing {name}"));
    let number = |name: &str| {
        doc.get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("task source missing {name}"))
    };
    Ok(TaskSource {
        id: source_id_of(&stored.id),
        provider: ProviderSlug::parse(field("provider")?).map_err(|error| anyhow!(error))?,
        connection_id: text(doc, "connection_id").map(str::to_string),
        name: text(doc, "name").map(str::to_string),
        enabled: doc.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        filter: serde_json::from_str::<FilterSpec>(field("filter")?)
            .context("invalid filter json")?,
        interval_secs: number("interval_secs")?,
        target: serde_json::from_str::<SourceTarget>(field("target")?)
            .context("invalid target json")?,
        max_tasks_per_fetch: u32::try_from(number("max_tasks_per_fetch")?)
            .context("invalid max_tasks_per_fetch")?,
        created_at: from_ms(
            doc.get("created_ms")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("task source missing created_ms"))?,
        )?,
        last_fetch_at: doc
            .get("last_fetch_ms")
            .and_then(Value::as_i64)
            .map(from_ms)
            .transpose()?,
        last_status: text(doc, "last_status").map(str::to_string),
    })
}

fn not_found(id: &str) -> anyhow::Error {
    anyhow!("Task source '{id}' not found")
}

/// The task-source store over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    pub(super) fn add_source(&self, source: &TaskSource) -> Result<TaskSource> {
        let id = key_of(&source.id)?;
        let doc = to_doc(source)?;
        self.0.run(|docs| async move {
            docs.put(SOURCES, &id, doc, Precondition::Absent)
                .await
                .map(|_| ())
        })?;
        self.get_source(&source.id)
    }

    pub(super) fn get_source(&self, id: &str) -> Result<TaskSource> {
        let key = key_of(id)?;
        let stored = self
            .0
            .run(|docs| async move { docs.get(SOURCES, &key).await })?
            .ok_or_else(|| not_found(id))?;
        to_source(&stored)
    }

    pub(super) fn list_sources(&self) -> Result<Vec<TaskSource>> {
        let stored = self.0.run(|docs| async move {
            let query = Query::all()
                .sort(Sort::asc("created_ms"))
                .sort(Sort::asc("_id"));
            docs.query_all(SOURCES, &query).await
        })?;
        stored.iter().map(to_source).collect()
    }

    pub(super) fn update_source(&self, id: &str, patch: TaskSourcePatch) -> Result<TaskSource> {
        // Validate against the current source first, so a mismatched filter
        // is reported as the SQL store reports it.
        apply_patch(&mut self.get_source(id)?, patch.clone())?;
        let key = key_of(id)?;
        let updated = self.0.run(|docs| async move {
            compare_and_swap(&docs, SOURCES, &key, |doc| {
                let stored = Versioned {
                    id: key.clone(),
                    version: tinystoragedrivers::Version::FIRST,
                    doc: doc.clone(),
                    meta: tinystoragedrivers::Meta::unknown(),
                };
                let mut source = to_source(&stored).ok()?;
                apply_patch(&mut source, patch.clone()).ok()?;
                to_doc(&source).ok()
            })
            .await
        })?;
        to_source(&updated.ok_or_else(|| not_found(id))?)
    }

    pub(super) fn remove_source(&self, id: &str) -> Result<()> {
        let key = key_of(id)?;
        let source_id = id.to_string();
        let removed = self.0.run(|docs| async move {
            // Ledger first: if the source delete then fails, the source is
            // still there (and merely re-ingests), instead of a ledger with no
            // owner that a later source with the same id would inherit.
            docs.delete_where(INGESTED, &Filter::eq("source_id", source_id))
                .await?;
            docs.delete(SOURCES, &key, Precondition::None).await
        })?;
        if removed {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    pub(super) fn record_fetch(
        &self,
        id: &str,
        finished_at: DateTime<Utc>,
        reason: FetchReason,
        status: &str,
    ) -> Result<()> {
        let key = key_of(id)?;
        let line = format!("{}: {status}", reason.as_str());
        let finished = finished_at.timestamp_millis();
        self.0.run(|docs| async move {
            compare_and_swap(&docs, SOURCES, &key, |doc| {
                let mut next = doc.clone();
                next["last_fetch_ms"] = json!(finished);
                next["last_status"] = json!(line);
                Some(next)
            })
            .await
            .map(|_| ())
        })
    }

    fn ledger_entry(&self, source_id: &str, external_id: &str) -> Result<Option<Versioned<Value>>> {
        let key = ingested_id(source_id, external_id);
        self.0
            .run(|docs| async move { docs.get(INGESTED, &key).await })
    }

    pub(super) fn is_ingested(
        &self,
        source_id: &str,
        external_id: &str,
        hash: &str,
    ) -> Result<bool> {
        Ok(self
            .ledger_entry(source_id, external_id)?
            .is_some_and(|stored| text(&stored.doc, "content_hash") == Some(hash)))
    }

    pub(super) fn was_ingested(&self, source_id: &str, external_id: &str) -> Result<bool> {
        Ok(self.ledger_entry(source_id, external_id)?.is_some())
    }

    pub(super) fn mark_ingested(&self, source_id: &str, task: &NormalizedTask) -> Result<()> {
        self.mark_ingested_at(source_id, task, Utc::now())
    }

    /// [`Self::mark_ingested`] with the ingestion time supplied.
    fn mark_ingested_at(
        &self,
        source_id: &str,
        task: &NormalizedTask,
        now: DateTime<Utc>,
    ) -> Result<()> {
        // The SQL ledger has a foreign key to the source; keep that parent
        // check so a fetch racing a removal does not leave an orphan entry.
        self.get_source(source_id)?;
        let key = ingested_id(source_id, &task.external_id);
        let doc = json!({
            "source_id": source_id,
            "external_id": task.external_id,
            "content_hash": content_hash(task),
            "title": task.title,
            "payload": serde_json::to_string(task).context("serialize ingested task payload")?,
            "ingested_ms": now.timestamp_millis(),
        });
        self.0.run(|docs| async move {
            docs.put(INGESTED, &key, doc, Precondition::None)
                .await
                .map(|_| ())
        })
    }

    pub(super) fn list_ingested_refs(&self, source_id: &str) -> Result<Vec<IngestedTaskRef>> {
        let query = Query::filter(Filter::eq("source_id", source_id))
            .sort(Sort::asc("ingested_ms"))
            .sort(Sort::asc("external_id"));
        self.0.run(|docs| async move {
            Ok(docs
                .query_all(INGESTED, &query)
                .await?
                .iter()
                .filter_map(|stored| {
                    Some(IngestedTaskRef {
                        external_id: text(&stored.doc, "external_id")?.to_string(),
                    })
                })
                .collect())
        })
    }

    pub(super) fn remove_ingested(&self, source_id: &str, external_id: &str) -> Result<bool> {
        let key = ingested_id(source_id, external_id);
        self.0
            .run(|docs| async move { docs.delete(INGESTED, &key, Precondition::None).await })
    }

    pub(super) fn list_ingested(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<NormalizedTask>> {
        let query =
            Query::filter(Filter::eq("source_id", source_id).and(Filter::exists("payload", true)))
                .sort(Sort::desc("ingested_ms"))
                .sort(Sort::desc("external_id"))
                .limit(limit.max(1));
        self.0.run(|docs| async move {
            let page = docs.query(INGESTED, &query).await?;
            page.items
                .iter()
                .map(|stored| {
                    let raw = text(&stored.doc, "payload").ok_or_else(|| {
                        StorageError::serialization(format!(
                            "ingested task {} has no payload",
                            stored.id
                        ))
                    })?;
                    serde_json::from_str(raw).map_err(|error| {
                        StorageError::serialization(format!(
                            "ingested task {} payload is not valid: {error}",
                            stored.id
                        ))
                    })
                })
                .collect::<Result<Vec<NormalizedTask>, StorageError>>()
        })
    }

    pub(super) fn clear_all(&self) -> Result<usize> {
        self.0.run(|docs| async move {
            let removed = docs.delete_where(SOURCES, &Filter::All).await?;
            docs.delete_where(INGESTED, &Filter::All).await?;
            Ok(usize::try_from(removed).unwrap_or(usize::MAX))
        })
    }
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
