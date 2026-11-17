//! What a pinned source's turns produced, on the `tinystoragedrivers`
//! document port.
//!
//! A turn of a pinned source ([`super::pinned`]) is recorded `running` before
//! its agent runs and `completed` (with the agent's final text) or `failed`
//! (with the error) when it ends. The pusher reads them over
//! `inbound.results` and acknowledges each with `inbound.result_ack`; an
//! acknowledged result is no longer listed, and every result is pruned once
//! it is older than its source's `result_ttl_days`.
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `inbound.results` | `Key::of([turn_id])` | one turn: its source, key, agent, events, status, text or error, and the message it ran with |
//!
//! Domain events only wake a subscriber; the text is read here, never carried
//! on the bus.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Filter, IndexSpec, Precondition, Query, Sort};

use crate::config::Config;
use crate::storage::documents::{compare_and_swap, Repo};
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

/// The results collection.
pub const RESULTS: &str = "inbound.results";
const DOMAIN: &str = "inbound::results";

/// What the inbound host keeps: the results of pinned sources' turns.
pub fn collection_specs() -> Vec<CollectionSpec> {
    vec![spec()]
}

fn spec() -> CollectionSpec {
    CollectionSpec::new(RESULTS)
        .index(IndexSpec::new("by_status", ["status", "created_ms"]))
        .index(IndexSpec::new("by_source", ["source", "created_ms"]))
}

/// Where a turn stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    /// The agent is running (or a previous process stopped while it was).
    Running,
    /// The agent answered; `text` holds its final reply.
    Completed,
    /// The agent failed for good; `error` says why.
    Failed,
}

impl ResultStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// One turn's result, as `inbound.results` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundResult {
    /// The queues' turn id.
    pub turn_id: String,
    /// The configured source.
    pub source: String,
    /// The key its events were batched by.
    pub key: String,
    /// The pinned agent that ran.
    pub agent_id: String,
    /// Where the turn stands.
    pub status: ResultStatus,
    /// The agent's final text, once completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The source's ids of the events the turn carried, in order.
    pub event_ids: Vec<String>,
    /// The events the prompt-injection guard kept out of the agent's prompt.
    #[serde(default)]
    pub blocked: Vec<BlockedEvent>,
    /// When the turn was first started, Unix epoch milliseconds.
    pub created_ms: u64,
    /// When it completed or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_ms: Option<u64>,
}

/// An event the prompt-injection guard kept out of a turn's prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedEvent {
    /// The source's id of the event.
    pub event_id: String,
    /// The guard's verdict and rule codes (`blocked: instruction_override`);
    /// never the event's text.
    pub reason: String,
}

/// A result as stored: the wire form, the message the agent was given (so a
/// run a previous process left unfinished can be run again) and whether the
/// pusher acknowledged it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// The wire form.
    #[serde(flatten)]
    pub result: InboundResult,
    /// The message the agent runs with.
    pub message: String,
    /// The pusher acknowledged the result.
    #[serde(default)]
    pub acked: bool,
}

impl Stored {
    /// A turn about to run.
    #[must_use]
    pub fn running(
        turn_id: &str,
        source: &str,
        key: &str,
        agent_id: &str,
        event_ids: Vec<String>,
        message: &str,
        created_ms: u64,
    ) -> Self {
        Self {
            result: InboundResult {
                turn_id: turn_id.to_owned(),
                source: source.to_owned(),
                key: key.to_owned(),
                agent_id: agent_id.to_owned(),
                status: ResultStatus::Running,
                text: None,
                error: None,
                event_ids,
                blocked: Vec::new(),
                created_ms,
                completed_ms: None,
            },
            message: message.to_owned(),
            acked: false,
        }
    }
}

impl Stored {
    /// With `blocked` recorded as kept out of the prompt.
    #[must_use]
    pub fn with_blocked(mut self, blocked: Vec<BlockedEvent>) -> Self {
        self.result.blocked = blocked;
        self
    }
}

/// Which results `inbound.results` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultQuery {
    /// Only this source's.
    pub source: Option<String>,
    /// Only those started at or after this time.
    pub since_ms: Option<u64>,
    /// Only those with this status.
    pub status: Option<ResultStatus>,
    /// At most this many.
    pub limit: usize,
}

impl Default for ResultQuery {
    fn default() -> Self {
        Self {
            source: None,
            since_ms: None,
            status: None,
            limit: 50,
        }
    }
}

fn decode(doc: &Value) -> Option<Stored> {
    serde_json::from_value(doc.clone())
        .map_err(|error| {
            tracing::warn!(%error, "[inbound::results] skipping an unreadable result");
        })
        .ok()
}

fn encode(stored: &Stored) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(stored)?)
}

/// The results store over one scoped document handle.
#[derive(Clone)]
pub struct Results(Repo);

impl Results {
    /// The store for `config`'s workspace, in the acting scope.
    ///
    /// # Errors
    ///
    /// The storage backend or scope cannot be opened.
    pub fn open(config: &Config) -> anyhow::Result<Self> {
        Ok(Self(Repo::current_for(
            &config.workspace_dir,
            DOMAIN,
            collection_specs,
        )?))
    }

    #[cfg(test)]
    pub(crate) fn over(scoped: &crate::storage::Storage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collection_specs))
    }

    /// Record `stored` unless its turn is already recorded; either way, the
    /// record as it stands.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn start(&self, stored: &Stored) -> anyhow::Result<Stored> {
        let id = key_of(&stored.result.turn_id)?;
        let doc = encode(stored)?;
        let fresh = stored.clone();
        self.0.run(|docs| async move {
            match docs.put(RESULTS, &id, doc, Precondition::Absent).await {
                Ok(_) => Ok(fresh),
                Err(error) if error.kind() == ErrorKind::Conflict => Ok(docs
                    .get(RESULTS, &id)
                    .await?
                    .and_then(|found| decode(&found.doc))
                    .unwrap_or(fresh)),
                Err(error) => Err(error),
            }
        })
    }

    /// The record of `turn_id`.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn get(&self, turn_id: &str) -> anyhow::Result<Option<Stored>> {
        let id = key_of(turn_id)?;
        self.0.run(|docs| async move {
            Ok(docs
                .get(RESULTS, &id)
                .await?
                .and_then(|found| decode(&found.doc)))
        })
    }

    /// End `turn_id` with the agent's text or its error, at `now_ms`; `None`
    /// when it was never started.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn finish(
        &self,
        turn_id: &str,
        outcome: Result<String, String>,
        now_ms: u64,
    ) -> anyhow::Result<Option<Stored>> {
        let id = key_of(turn_id)?;
        self.0.run(|docs| async move {
            let written = compare_and_swap(&docs, RESULTS, &id, |doc| {
                let mut stored = decode(doc)?;
                match &outcome {
                    Ok(text) => {
                        stored.result.status = ResultStatus::Completed;
                        stored.result.text = Some(text.clone());
                        stored.result.error = None;
                    }
                    Err(error) => {
                        stored.result.status = ResultStatus::Failed;
                        stored.result.error = Some(error.clone());
                    }
                }
                stored.result.completed_ms = Some(now_ms);
                serde_json::to_value(stored).ok()
            })
            .await?;
            Ok(written.and_then(|found| decode(&found.doc)))
        })
    }

    /// Unacknowledged results matching `query`, oldest first.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn list(&self, query: &ResultQuery) -> anyhow::Result<Vec<InboundResult>> {
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let filter = [
            Some(Filter::eq("acked", false)),
            query.source.as_deref().map(|s| Filter::eq("source", s)),
            query.since_ms.map(|ms| Filter::gte("created_ms", ms)),
            query.status.map(|s| Filter::eq("status", s.as_str())),
        ]
        .into_iter()
        .flatten()
        .reduce(Filter::and)
        .unwrap_or(Filter::All);
        let query = Query::filter(filter)
            .sort(Sort::asc("created_ms"))
            .limit(query.limit);
        self.0.run(|docs| async move {
            Ok(docs
                .query(RESULTS, &query)
                .await?
                .items
                .iter()
                .filter_map(|found| decode(&found.doc))
                .map(|stored| stored.result)
                .collect())
        })
    }

    /// Every turn still recorded `running`, oldest first.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn running(&self) -> anyhow::Result<Vec<Stored>> {
        let query = Query::filter(Filter::eq("status", ResultStatus::Running.as_str()))
            .sort(Sort::asc("created_ms"));
        self.0.run(|docs| async move {
            Ok(docs
                .query_all(RESULTS, &query)
                .await?
                .iter()
                .filter_map(|found| decode(&found.doc))
                .collect())
        })
    }

    /// Acknowledge `turn_id`'s result; `false` when there is none.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn ack(&self, turn_id: &str) -> anyhow::Result<bool> {
        let id = key_of(turn_id)?;
        self.0.run(|docs| async move {
            let written = compare_and_swap(&docs, RESULTS, &id, |doc| {
                let mut next = doc.clone();
                next["acked"] = Value::Bool(true);
                Some(next)
            })
            .await?;
            Ok(written.is_some())
        })
    }

    /// Delete every result older at `now_ms` than its source's TTL
    /// (`ttl_ms(source)`). Returns how many.
    ///
    /// # Errors
    ///
    /// The backend failed.
    pub fn prune(
        &self,
        now_ms: u64,
        ttl_ms: impl Fn(&str) -> u64 + Send + 'static,
    ) -> anyhow::Result<usize> {
        self.0.run(|docs| async move {
            let mut pruned = 0;
            for found in docs.query_all(RESULTS, &Query::all()).await? {
                let Some(stored) = decode(&found.doc) else {
                    continue;
                };
                let age = now_ms.saturating_sub(stored.result.created_ms);
                if age > ttl_ms(&stored.result.source) {
                    match docs.delete(RESULTS, &found.id, found.unchanged()).await {
                        Ok(_) => pruned += 1,
                        Err(error) if error.kind() == ErrorKind::Conflict => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            Ok(pruned)
        })
    }
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;
