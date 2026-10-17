//! What the module asks of the host while it works on a remix: one
//! completion, one Jev evaluation, one web search, and the learned-state
//! documents ([`super::docs`]).
//!
//! Every callback carries the correlation id of the remix it belongs to (the
//! module echoes the one the host put in the request). A [`Registry`] entry
//! per id holds that remix's workspace, so remixes for different workspaces
//! run side by side and a callback with no live id is refused. An entry lives
//! until the module reports the request finished with `End` (background
//! research can outlast the call), or is swept after [`MAX_AGE`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tinyinference_decisions::{Client, EvaluationRequest};
use tinymemes_bus::{
    host_methods, CompleteRequest, EvaluateEnvelope, HostFailure, HostResult, IndexKind,
    LoadIndexRequest, SaveIndexRequest, SearchRequest,
};

use super::{docs, inference, jev, search};
use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};

/// How long an entry may live without its `End`.
const MAX_AGE: Duration = Duration::from_secs(15 * 60);

/// The workspace one remix runs for, and who it runs for.
///
/// Callbacks arrive on the module bus, outside the task that asked for the
/// remix, so the caller's context is captured when the remix is registered and
/// re-entered for each callback ([`Session::enter`]). A remix acts for the
/// host, never for an agent, so there is no memory identity to carry.
/// Only the caller's own scope counts, so in SaaS an unscoped caller's
/// callbacks fail closed instead of acting as the operator.
pub(super) struct Session {
    config: Config,
    jev: tokio::sync::OnceCell<Option<Arc<Client>>>,
    context: Option<Arc<CoreContext>>,
}

impl Session {
    /// The dispatch context the registering caller runs under; the host's own
    /// when it had none (a single-user process), which fails closed in SaaS.
    pub(super) fn dispatch_ctx(&self) -> DispatchCtx {
        match self.context.clone() {
            Some(context) => DispatchCtx::new(context),
            None => DispatchCtx::host_default(),
        }
    }

    pub(super) fn new(ctx: &DispatchCtx, config: &Config) -> Self {
        Self {
            config: config.clone(),
            jev: tokio::sync::OnceCell::new(),
            // Only the caller's own context is carried. Without one, a
            // single-user process falls back to its default context on the
            // callback side as anywhere else, and SaaS fails closed.
            context: ctx.core().cloned(),
        }
    }

    /// Run `fut` as the registering caller.
    pub(super) async fn enter<F: Future>(&self, fut: F) -> F::Output {
        let fut = Box::pin(fut);
        match self.context.clone() {
            Some(context) => CoreContext::scope(context, fut).await,
            None => fut.await,
        }
    }
}

struct Entry {
    session: Arc<Session>,
    registered: Instant,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Mutex::default);

fn registry() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// A registered remix. Dropping it forgets the remix unless it was
/// [`kept`](Self::keep_for_background) for research that runs on after the
/// call returns.
pub(super) struct Registration {
    correlation: String,
    keep: bool,
}

impl Registration {
    /// The id to put in the request.
    pub(super) fn correlation(&self) -> &str {
        &self.correlation
    }

    /// The call succeeded: leave the entry for the module's `End`.
    pub(super) fn keep_for_background(mut self) {
        self.keep = true;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.keep {
            registry().remove(&self.correlation);
        }
    }
}

/// Register a remix for `config`'s workspace under a fresh correlation id.
pub(super) fn register(ctx: &DispatchCtx, config: &Config) -> Registration {
    let correlation = uuid::Uuid::new_v4().to_string();
    let mut sessions = registry();
    sessions.retain(|_, entry| entry.registered.elapsed() < MAX_AGE);
    sessions.insert(
        correlation.clone(),
        Entry {
            session: Arc::new(Session::new(ctx, config)),
            registered: Instant::now(),
        },
    );
    Registration {
        correlation,
        keep: false,
    }
}

/// The session a callback answers for, if its remix is still registered.
pub(super) fn session(correlation: &str) -> Option<Arc<Session>> {
    registry()
        .get(correlation)
        .map(|entry| entry.session.clone())
}

/// The module finished this remix: forget it.
pub(super) fn end(correlation: &str) {
    if registry().remove(correlation).is_some() {
        log::debug!("[tinymemes] remix ended correlation={correlation}");
    }
}

/// How many registered remixes belong to `workspace`; tests use a unique
/// workspace so parallel tests cannot disturb the count.
#[cfg(test)]
pub(super) fn live_for(workspace: &std::path::Path) -> usize {
    registry()
        .values()
        .filter(|entry| entry.session.config.workspace_dir == workspace)
        .count()
}

/// The correlation id a callback payload carries (`[ { "correlation": .. } ]`).
pub(super) fn correlation_of(payload: &Value) -> Option<&str> {
    payload
        .as_array()?
        .first()?
        .get("correlation")?
        .as_str()
        .filter(|c| !c.is_empty())
}

fn args<T: DeserializeOwned>(value: Value) -> Result<T, tinybus::Error> {
    let (request,): (T,) = serde_json::from_value(value)
        .map_err(|_| tinybus::Error::failed("invalid tinymemes host request"))?;
    Ok(request)
}

fn reply<T: serde::Serialize>(result: HostResult<T>) -> Result<Value, tinybus::Error> {
    serde_json::to_value(result)
        .map_err(|_| tinybus::Error::failed("invalid tinymemes host response"))
}

/// Answer one host callback for `session`.
pub(super) async fn dispatch(
    session: &Session,
    member: &str,
    payload: Value,
) -> Result<Value, tinybus::Error> {
    log::debug!("[tinymemes] host callback member={member}");
    session.enter(answer(session, member, payload)).await
}

async fn answer(session: &Session, member: &str, payload: Value) -> Result<Value, tinybus::Error> {
    match member {
        host_methods::COMPLETE => reply(complete(session, args(payload)?).await),
        host_methods::EVALUATE => reply(evaluate(session, args(payload)?).await),
        host_methods::SEARCH => reply(search(session, args(payload)?).await),
        host_methods::LOAD_INDEX => reply(load_index(session, args(payload)?).await),
        host_methods::SAVE_INDEX => reply(save_index(session, args(payload)?).await),
        // The remixed text is returned to the caller, which delivers it
        // itself; nothing is handed over through the module.
        host_methods::DELIVER => reply(HostResult::Ok(())),
        host_methods::END => {
            if let Some(correlation) = correlation_of(&payload) {
                end(correlation);
            }
            reply(HostResult::Ok(()))
        }
        _ => Err(tinybus::Error::failed("unknown tinymemes host operation")),
    }
}

async fn complete(session: &Session, request: CompleteRequest) -> HostResult<String> {
    inference::complete(
        &session.dispatch_ctx(),
        &session.config,
        &request.system,
        &request.user,
    )
    .await
    .map_err(|error| {
        log::warn!("[tinymemes] complete callback failed: {error}");
        HostFailure::Backend
    })
}

async fn evaluate(session: &Session, envelope: EvaluateEnvelope) -> HostResult<EvaluateEnvelope> {
    let Some(client) = session
        .jev
        .get_or_init(|| async { jev::managed(&session.config) })
        .await
    else {
        log::debug!("[tinymemes] evaluate callback: managed jev unavailable");
        return Err(HostFailure::Unsupported);
    };
    let request: EvaluationRequest =
        serde_json::from_value(envelope.body).map_err(|_| HostFailure::Unsupported)?;
    let result = client.evaluate(&request).await.map_err(|error| {
        log::warn!("[tinymemes] evaluate callback failed: {error}");
        HostFailure::Backend
    })?;
    let body = serde_json::to_value(result.response).map_err(|_| HostFailure::Backend)?;
    Ok(EvaluateEnvelope {
        correlation: String::new(),
        body,
    })
}

async fn search(
    session: &Session,
    request: SearchRequest,
) -> HostResult<Vec<tinymemes_bus::SearchResult>> {
    let Some(backend) =
        search::OpenHumanSearch::available(&session.dispatch_ctx(), &session.config)
    else {
        return Err(HostFailure::Unsupported);
    };
    backend
        .search(&request.query, request.max_results as usize)
        .await
        .map_err(|error| {
            // The query is derived from the chat, so only the error is logged.
            log::warn!("[tinymemes] search callback failed: {error}");
            HostFailure::Backend
        })
}

async fn load_index(session: &Session, request: LoadIndexRequest) -> HostResult<Option<String>> {
    let config = session.config.clone();
    let kind: IndexKind = request.kind;
    match session
        .dispatch_ctx()
        .spawn_blocking(move || docs::load(&config, kind))
        .await
    {
        Ok(Ok(document)) => Ok(document),
        Ok(Err(error)) => {
            log::warn!("[tinymemes] cannot read index: {error}");
            Err(HostFailure::Backend)
        }
        Err(error) => {
            log::warn!("[tinymemes] index read task failed: {error}");
            Err(HostFailure::Backend)
        }
    }
}

async fn save_index(session: &Session, request: SaveIndexRequest) -> HostResult<()> {
    let config = session.config.clone();
    match session
        .dispatch_ctx()
        .spawn_blocking(move || docs::save(&config, request.kind, request.json))
        .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            log::warn!("[tinymemes] cannot save index: {error}");
            Err(HostFailure::Backend)
        }
        Err(error) => {
            log::warn!("[tinymemes] index write task failed: {error}");
            Err(HostFailure::Backend)
        }
    }
}

#[cfg(test)]
#[path = "callbacks_tests.rs"]
mod tests;
