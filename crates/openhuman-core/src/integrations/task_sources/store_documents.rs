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
//! | `task_sources` | source id | one configured source |
//! | `ingested_tasks` | `(source_id, external_id)` | one dedup ledger entry |
//!
//! `filter`, `target` and the task `payload` are JSON strings, as in SQLite,
//! so a MongoDB-backed port never meets an arbitrary key. Epoch-millisecond
//! fields (`created_ms`, `ingested_ms`) carry the ordering. Removing a source
//! removes its ledger entries, which the SQL schema did with a cascade.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use tinystoragedrivers::{CollectionSpec, Filter, IndexSpec, Precondition, Query, Sort, Versioned};

use super::store::{apply_patch, content_hash, IngestedTaskRef};
use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};
use crate::config::Config;
use crate::integrations::composio::contract::NormalizedTask;
use crate::storage::documents::{compare_and_swap, text, Repo};
use crate::storage::{DocumentStoreExt, StorageError};

pub(super) const SOURCES: &str = "task_sources";
pub(super) const INGESTED: &str = "ingested_tasks";
const DOMAIN: &str = "task_sources::store";

/// The legacy `sources.db`, as the storage importer reads it.
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "task_sources",
        db: "task_sources/sources.db",
        collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(SOURCES).index(IndexSpec::new("by_created", ["created_ms"])),
        CollectionSpec::new(INGESTED)
            .index(IndexSpec::new("by_source", ["source_id", "ingested_ms"])),
    ]
}

/// The document store for this call: the storage backend, in the acting scope.
pub(super) fn current(config: &Config) -> Result<Docs> {
    Ok(Docs(Repo::current_for(
        &config.workspace_dir,
        DOMAIN,
        collections,
    )?))
}

/// The ledger id for one `(source, external task)`; the length prefix keeps
/// `("a/b", "c")` and `("a", "b/c")` apart.
pub(super) fn ingested_id(source_id: &str, external_id: &str) -> String {
    format!("{}:{source_id}/{external_id}", source_id.len())
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
    doc.insert("created_at".into(), json!(source.created_at.to_rfc3339()));
    doc.insert(
        "created_ms".into(),
        json!(source.created_at.timestamp_millis()),
    );
    let optional = [
        ("connection_id", source.connection_id.clone()),
        ("name", source.name.clone()),
        (
            "last_fetch_at",
            source.last_fetch_at.map(|at| at.to_rfc3339()),
        ),
        ("last_status", source.last_status.clone()),
    ];
    for (field, value) in optional {
        if let Some(value) = value {
            doc.insert(field.into(), json!(value));
        }
    }
    Ok(Value::Object(doc))
}

fn parse_time(raw: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 timestamp in task source: {raw}"))?
        .with_timezone(&Utc))
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
        id: stored.id.clone(),
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
        created_at: parse_time(field("created_at")?)?,
        last_fetch_at: text(doc, "last_fetch_at").map(parse_time).transpose()?,
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
        let id = source.id.clone();
        let doc = to_doc(source)?;
        self.0.run(|docs| async move {
            docs.put(SOURCES, &id, doc, Precondition::Absent)
                .await
                .map(|_| ())
        })?;
        self.get_source(&source.id)
    }

    pub(super) fn get_source(&self, id: &str) -> Result<TaskSource> {
        let key = id.to_string();
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
        let key = id.to_string();
        let updated = self.0.run(|docs| async move {
            compare_and_swap(&docs, SOURCES, &key, |doc| {
                let stored = Versioned {
                    id: key.clone(),
                    version: tinystoragedrivers::Version::FIRST,
                    doc: doc.clone(),
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
        let key = id.to_string();
        let removed = self.0.run(|docs| async move {
            // Ledger first: if the source delete then fails, the source is
            // still there (and merely re-ingests), instead of a ledger with no
            // owner that a later source with the same id would inherit.
            docs.delete_where(INGESTED, &Filter::eq("source_id", key.clone()))
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
        let key = id.to_string();
        let line = format!("{}: {status}", reason.as_str());
        let finished = finished_at.to_rfc3339();
        self.0.run(|docs| async move {
            compare_and_swap(&docs, SOURCES, &key, |doc| {
                let mut next = doc.clone();
                next["last_fetch_at"] = json!(finished);
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
            "ingested_at": now.to_rfc3339(),
            "ingested_ms": now.timestamp_millis(),
            // Breaks ties between arrivals in one millisecond.
            "ingested_ns": now.timestamp_nanos_opt().unwrap_or(i64::MAX),
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
            .sort(Sort::asc("ingested_ns"))
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
                .sort(Sort::desc("ingested_ns"))
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
