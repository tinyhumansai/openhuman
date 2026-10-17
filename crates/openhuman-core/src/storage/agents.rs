//! The agents that keep records in their own storage scope, and a way for
//! background work to visit each of them.
//!
//! Work done inside an agent's turn runs under that agent's `CoreContext`
//! (`session_agent`), so with a storage backend installed its cron jobs,
//! flows, approvals and the rest land in that agent's scope. Background work
//! — the cron scheduler, pollers, boot sweeps — runs under the process
//! default context, which names no agent, and on its own would only ever see
//! the `local` scope. [`for_each_scope`] closes that gap: it runs a step once
//! for `local` and once under each known agent's context.
//!
//! An agent is known when it is live in this process
//! (`core::runtime::AgentContextRegistry`, which embed agents register on
//! build) — its own context is used, with its configuration, policy and
//! tools — or when an earlier process recorded its id in the backend's
//! `local` scope (`agents.registry`, written by [`record`] when the agent
//! registers), so a restart still visits agents the host has not re-created
//! yet; those are visited under the default context acting for them
//! (`CoreContext::for_agent`).
//!
//! Without a backend [`for_each_agent`] visits nothing: the SQLite stores
//! these loops read do not split by agent. In SaaS mode the process has no
//! `local` scope and per-user background work is driven by
//! `profiles::background`, so agent ids are not recorded and
//! [`for_each_scope`] skips `local`.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::{json, Value};
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, Precondition, Query, Scope};

use super::schema_support::{key_of, owner};
use super::{block_on, installed, DocumentStoreExt, StorageBackend};
use crate::core::runtime::{AgentContextRegistry, CoreContext, DispatchCtx};

/// The `local`-scope collection recording which agents have their own scope.
pub(super) const AGENTS: &str = "agents.registry";
/// The v1 name, `{}` per agent under the agent id.
const LEGACY_AGENTS: &str = "storage_agents";

/// What the agent registry keeps and the step from the v1 collection: the
/// document id is `Key::of([agent id])` and the body names the agent, so a
/// listing needs no unescaping.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("agents"))
        .collection_with(2, CollectionSpec::new(AGENTS), ScopeClass::Tenant)
        .step(Step::new("registry_v2", AGENTS, 1, 2, registry_v2).source(LEGACY_AGENTS))
}

fn registry_v2(batch: Vec<SourceRecord>) -> tinystoragedrivers::Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            Ok(TargetRecord::new(
                AGENTS,
                key_of(&record.id)?,
                json!({ "agent_id": record.id }),
            )
            .created_ms(record.meta.created_ms))
        })
        .collect()
}

/// `(backend, agent id)` pairs this process has already recorded, the
/// backend identified by its address (see [`backend_key`]).
static RECORDED: LazyLock<Mutex<HashSet<(usize, String)>>> = LazyLock::new(Default::default);

/// Identifies `backend` within this process. [`reset_recorded`] runs on every
/// install and clear, so an address reused by a later backend never inherits
/// an earlier one's records.
fn backend_key(backend: &Arc<dyn StorageBackend>) -> usize {
    Arc::as_ptr(backend).cast::<()>() as usize
}

/// Forgets which agents were recorded, so the next [`record`] writes them to
/// the backend now installed (or removed). Called by [`super::install`] and
/// [`super::clear`]: the record cache describes one backend, not the process.
pub(super) fn reset_recorded() {
    RECORDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

/// Records every live agent in the backend — for agents registered before
/// the host installed its backend. Called by [`super::install`].
pub(super) fn record_live() {
    for (agent, _) in AgentContextRegistry::live() {
        record(&agent);
    }
}

/// Writes `agent` to the backend's `storage_agents` collection, once per
/// process. `AgentContextRegistry::register` calls it for every agent. Best
/// effort: a failure is logged and retried on the next registration, and
/// only costs a restarted process its visits to that agent until the agent
/// registers again.
pub fn record(agent: &str) {
    if crate::core::runtime::mode::is_saas() {
        return;
    }
    let Some(backend) = installed() else {
        return;
    };
    record_in(backend, agent);
}

/// [`record`] against an explicit `backend`.
fn record_in(backend: Arc<dyn StorageBackend>, agent: &str) {
    let key = (backend_key(&backend), agent.to_string());
    if RECORDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&key)
    {
        return;
    }
    let id = agent.to_string();
    let result = block_on(async move {
        let docs = Arc::clone(backend.for_scope(&Scope::local())?.documents());
        docs.ensure_collection(&CollectionSpec::new(AGENTS)).await?;
        docs.put(
            AGENTS,
            &key_of(&id)?,
            json!({ "agent_id": id }),
            Precondition::None,
        )
        .await?;
        Ok(())
    });
    match result {
        Ok(()) => {
            RECORDED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key);
            tracing::debug!(%agent, "[storage::agents] recorded agent scope");
        }
        Err(error) => {
            tracing::warn!(%agent, %error, "[storage::agents] could not record agent scope");
        }
    }
}

/// Agent ids recorded in the backend by this or an earlier process.
fn recorded(backend: Arc<dyn StorageBackend>) -> Vec<String> {
    let result = block_on(async move {
        let docs = Arc::clone(backend.for_scope(&Scope::local())?.documents());
        docs.ensure_collection(&CollectionSpec::new(AGENTS)).await?;
        let stored = docs.query_all(AGENTS, &Query::all()).await?;
        Ok(stored
            .into_iter()
            .filter_map(|doc| {
                doc.doc
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>())
    });
    result.unwrap_or_else(|error| {
        tracing::warn!(%error, "[storage::agents] could not list recorded agent scopes");
        Vec::new()
    })
}

/// Every agent background work should visit, each with the context to visit
/// it under: the live ones (`AgentContextRegistry`), then — with a storage
/// backend installed, outside SaaS mode — every agent recorded by this or an
/// earlier process, under the process default context acting for it (never
/// whichever agent happens to be ambient, whose policy and configuration
/// are not the recorded agent's).
pub fn contexts() -> Vec<(String, Arc<CoreContext>)> {
    let backend = if crate::core::runtime::mode::is_saas() {
        None
    } else {
        installed()
    };
    contexts_in(backend, CoreContext::default_context().as_ref())
}

/// [`contexts`] with the backend whose recorded agents are visited, and the
/// context that acts for an agent with no live one, made explicit (`None`
/// visits live contexts only).
fn contexts_in(
    backend: Option<Arc<dyn StorageBackend>>,
    fallback: Option<&Arc<CoreContext>>,
) -> Vec<(String, Arc<CoreContext>)> {
    let mut contexts: BTreeMap<String, Arc<CoreContext>> =
        AgentContextRegistry::live().into_iter().collect();
    if let (Some(backend), Some(fallback)) = (backend, fallback) {
        for agent in recorded(backend) {
            contexts
                .entry(agent.clone())
                .or_insert_with(|| fallback.for_agent(&agent));
        }
    }
    contexts.into_iter().collect()
}

/// The context to act for `agent` under: its live context when one exists,
/// else — outside SaaS mode — the process default context acting for it
/// (`CoreContext::for_agent`). SaaS acts only through a user's own live
/// context.
pub fn context_for(agent: &str) -> Option<Arc<CoreContext>> {
    AgentContextRegistry::get(agent).or_else(|| {
        if crate::core::runtime::mode::is_saas() {
            return None;
        }
        CoreContext::default_context().map(|default| default.for_agent(agent))
    })
}

/// The context the `local` scope runs under for a caller holding `ctx`: the
/// process default context when the caller is an embedded agent outside any
/// SaaS profile (it must see the local records, not its own), `ctx` itself
/// otherwise. A SaaS profile (or a SaaS task with no scope) never switches to
/// the process default context, which is the operator's.
fn local_context(ctx: &DispatchCtx) -> DispatchCtx {
    let acting = ctx
        .tenant()
        .is_ok_and(|tenant| tenant.profile.is_none() && tenant.agent.is_some());
    if acting && !crate::core::runtime::mode::is_saas() {
        return DispatchCtx::process_default();
    }
    ctx.clone()
}

/// Runs `fut` in the `local` scope, under `local` when it is bound.
async fn in_local<F: Future>(local: &DispatchCtx, fut: F) -> F::Output {
    match local.core() {
        Some(core) => CoreContext::scope(Arc::clone(core), fut).await,
        None => fut.await,
    }
}

/// Runs `fut` acting for `agent` — background work that learned whose record
/// it is handling (a device's pairing agent, a flow's owner) re-enters that
/// agent's scope — or in the `local` scope for `None`.
///
/// Returns `None`, without running `fut`, when `agent` is named but no
/// context can act for it (SaaS with the agent gone, or no booted core):
/// running it anywhere else would act outside the agent's scope.
pub async fn within_agent<F: Future>(
    ctx: &DispatchCtx,
    agent: Option<&str>,
    fut: F,
) -> Option<F::Output> {
    within_agent_ctx(ctx, agent, |_| fut).await
}

/// [`within_agent`] for work that takes the context it acts under: `step` is
/// handed the dispatch context of the agent's scope (the host's own for
/// `None`, which in SaaS is unbound: work that lost its tenant never acts as
/// the operator).
pub async fn within_agent_ctx<F, Fut>(
    ctx: &DispatchCtx,
    agent: Option<&str>,
    step: F,
) -> Option<Fut::Output>
where
    F: FnOnce(DispatchCtx) -> Fut,
    Fut: Future,
{
    let Some(agent) = agent else {
        let local = local_context(ctx);
        return Some(in_local(&local, step(local.clone())).await);
    };
    match context_for(agent) {
        Some(context) => {
            let dispatch = DispatchCtx::new(Arc::clone(&context));
            Some(CoreContext::scope(context, step(dispatch)).await)
        }
        None => {
            tracing::warn!(%agent, "[storage::agents] no context can act for the agent; skipped");
            None
        }
    }
}

/// A scope's lookup failed, and no scope reported the record: its owner is
/// unknown, so the work is not done rather than done in the wrong scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupFailed {
    /// The scope whose lookup failed (`None` = `local`).
    pub agent: Option<String>,
    /// Why.
    pub error: String,
}

impl std::fmt::Display for LookupFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "lookup failed in scope {}: {}",
            self.agent.as_deref().unwrap_or("local"),
            self.error
        )
    }
}

/// The scope a record lives in, for background work that holds only its id
/// (an event naming a flow, a job, a device): the first scope — `local`
/// first, then each known agent ([`for_each_scope`]) — where `probe` reports
/// it. `Ok(Some(None))` is `local`, `Ok(Some(Some(agent)))` an agent,
/// `Ok(None)` nowhere.
///
/// # Errors
///
/// [`LookupFailed`] when no scope reported the record and at least one
/// scope's probe failed.
pub async fn find_owner<F, Fut>(
    ctx: &DispatchCtx,
    label: &str,
    probe: F,
) -> Result<Option<Option<String>>, LookupFailed>
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    decide(for_each_scope(ctx, label, probe).await)
}

/// [`find_owner`]'s verdict over each scope's probe result.
///
/// # Errors
///
/// As [`find_owner`].
pub fn decide(
    lookups: Vec<(Option<String>, Result<bool, String>)>,
) -> Result<Option<Option<String>>, LookupFailed> {
    let mut failure = None;
    for (agent, lookup) in lookups {
        match lookup {
            Ok(true) => return Ok(Some(agent)),
            Ok(false) => {}
            Err(error) => {
                failure.get_or_insert(LookupFailed { agent, error });
            }
        }
    }
    failure.map_or(Ok(None), Err)
}

/// Runs `step` for every storage scope background work must cover: once in
/// the `local` scope (outside SaaS mode; under the default context even when
/// the caller is acting for an agent), then once per known agent
/// ([`for_each_agent`]). Each result is returned with the agent it ran for
/// (`None` for `local`).
///
/// `label` names the caller in logs.
pub async fn for_each_scope<T, F, Fut>(
    ctx: &DispatchCtx,
    label: &str,
    step: F,
) -> Vec<(Option<String>, T)>
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: Future<Output = T>,
{
    let mut results = Vec::new();
    if !crate::core::runtime::mode::is_saas() {
        let local = local_context(ctx);
        results.push((None, in_local(&local, step(local.clone())).await));
    }
    for (agent, value) in for_each_agent(label, step).await {
        results.push((Some(agent), value));
    }
    results
}

/// Runs `step` once under each known agent's context, one after another —
/// only when a storage backend is installed, since without one the stores do
/// not split by agent and the `local` pass already covers everything.
pub async fn for_each_agent<T, F, Fut>(label: &str, step: F) -> Vec<(String, T)>
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: Future<Output = T>,
{
    if installed().is_none() {
        return Vec::new();
    }
    visit(label, contexts(), step).await
}

/// [`for_each_scope`], visiting only live agents (`AgentContextRegistry`):
/// for work that acts as the agent — runs its flows, fetches with its
/// connections — and so needs the agent's own configuration and tools, which
/// a recorded agent's stand-in context (`CoreContext::for_agent`) lacks.
pub async fn for_each_live_scope<T, F, Fut>(
    ctx: &DispatchCtx,
    label: &str,
    step: F,
) -> Vec<(Option<String>, T)>
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: Future<Output = T>,
{
    let mut results = Vec::new();
    if !crate::core::runtime::mode::is_saas() {
        let local = local_context(ctx);
        results.push((None, in_local(&local, step(local.clone())).await));
    }
    if installed().is_none() {
        return results;
    }
    for (agent, value) in visit(label, AgentContextRegistry::live(), step).await {
        results.push((Some(agent), value));
    }
    results
}

/// Runs `step` under each of `contexts`, building each step inside its
/// agent's scope, so anything it reads while being set up is the agent's.
async fn visit<T, F, Fut>(
    label: &str,
    contexts: Vec<(String, Arc<CoreContext>)>,
    step: F,
) -> Vec<(String, T)>
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: Future<Output = T>,
{
    let mut results = Vec::new();
    for (agent, context) in contexts {
        tracing::trace!(%agent, label, "[storage::agents] visiting agent scope");
        let dispatch = DispatchCtx::new(Arc::clone(&context));
        let value = CoreContext::scope(context, async { step(dispatch).await }).await;
        results.push((agent, value));
    }
    results
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
