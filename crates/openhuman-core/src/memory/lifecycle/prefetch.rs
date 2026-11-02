//! Best-effort, scoped automatic memory refresh. By default turns only read
//! completed packs; a host that sets `[memory] recall_each_turn_ms` has each
//! turn wait up to that long for the pack recalled for its own message
//! ([`RecallEachTurn`]). All engine work and date extraction run in bounded
//! background workers either way.
//! Packs live in memory for one minute and are never persisted by this adapter.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::future::BoxFuture;
use tinymemory_tools::prefetch::{
    Limits, PrefetchRuntime, TurnCache, TurnCacheRegistry, WorkerPool,
};

use crate::config::Config;
use crate::core::runtime::{slot_for, DispatchCtx};
use tinymemory_tools::layout::identity::ResolvedIdentity;

use super::hooks::{self, TurnPack};

static WORKERS: LazyLock<WorkerPool> = LazyLock::new(|| WorkerPool::new(8));
static CACHES: LazyLock<TurnCacheRegistry<Key>> = LazyLock::new(TurnCacheRegistry::default);

type Work = BoxFuture<'static, Option<TurnPack>>;

/// The host executor the prefetcher's workers run on: the caller's core
/// context and memory identity ride along with every spawned task.
struct HostRuntime;

impl PrefetchRuntime for HostRuntime {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        drop(DispatchCtx::ambient().spawn(task));
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    workspace: PathBuf,
    config: u64,
    identity: ResolvedIdentity,
    thread: String,
    compaction: bool,
}

impl Key {
    /// `config_fingerprint` is supplied by the host so the cache key can
    /// later consume the host's config generation directly (e.g. TurnCache).
    fn with_fingerprint(
        config: &Config,
        config_fingerprint: u64,
        identity: &ResolvedIdentity,
        thread: &str,
        compaction: bool,
    ) -> Self {
        Self {
            workspace: config.workspace_dir.clone(),
            config: config_fingerprint,
            identity: identity.clone(),
            thread: thread.to_owned(),
            compaction,
        }
    }

    #[cfg(test)]
    fn new(config: &Config, identity: &ResolvedIdentity, thread: &str, compaction: bool) -> Self {
        Self::with_fingerprint(
            config,
            memory_config_fingerprint(config),
            identity,
            thread,
            compaction,
        )
    }
}

/// Fingerprints only memory settings that affect recall and its sources. The
/// full Config is large and unrelated settings must not invalidate turn packs.
fn memory_config_fingerprint(config: &Config) -> u64 {
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    config.memory.hash(&mut fingerprint);
    config.config_path.hash(&mut fingerprint);
    config.action_dir.hash(&mut fingerprint);
    config.api_url.hash(&mut fingerprint);
    config.inference_url.hash(&mut fingerprint);
    config.api_key.hash(&mut fingerprint);
    config.components.memory.hash(&mut fingerprint);
    config.default_model.hash(&mut fingerprint);
    config.chat_provider.hash(&mut fingerprint);
    config.memory_provider.hash(&mut fingerprint);
    config.model_routes.hash(&mut fingerprint);
    config.time_zone().hash(&mut fingerprint);
    fingerprint.finish()
}

/// `slot_for` constructs a per-context default, while the actual cache and
/// cross-context weak registry policy belong to TinyMemory.
struct Cache(Arc<TurnCache<Key>>);

impl Default for Cache {
    fn default() -> Self {
        Self(Arc::new(
            TurnCache::new(Arc::new(HostRuntime))
                .with_limits(Limits::default())
                .with_workers(WORKERS.clone()),
        ))
    }
}

fn cache(ctx: &DispatchCtx) -> Arc<Cache> {
    let cache = slot_for::<Cache>(ctx);
    CACHES.register(&cache.0);
    cache
}

/// Discards completed packs for this workspace, across its agent contexts.
/// Pending conversation logs are preserved; results of reads already in flight
/// cannot republish after invalidation. Call after
/// successful memory writes and erasure; no engine work runs here.
pub(crate) fn invalidate(config: &Config) {
    CACHES.invalidate_where(|key| key.workspace == config.workspace_dir);
}

/// Clears all automatic packs after engine or credential invalidation. Old
/// in-flight completions cannot recreate entries they no longer own.
pub(crate) fn invalidate_all() {
    CACHES.invalidate_all();
}

fn completed(cache: &Cache, key: &Key) -> Option<TurnPack> {
    cache.0.completed(key).map(TurnPack::from_native)
}

fn enqueue(cache: Arc<Cache>, key: Key, work: Work) {
    tracing::debug!(thread_id = %key.thread, agent_id = %key.identity.agent_id, compaction = key.compaction, "[memory:prefetch] enqueue refresh");
    cache.0.enqueue(
        key,
        Box::pin(async move { work.await.map(|pack| pack.native) }),
    );
}

/// Decides which recalled hits a turn must not be shown, for a host that
/// already puts them in front of the model some other way.
pub(crate) type RecallExclude = Arc<dyn Fn(&tinymemory_api::Hit) -> bool + Send + Sync>;

/// Recall for the current message, waited on for at most `timeout`.
#[derive(Clone)]
pub(crate) struct RecallEachTurn {
    timeout: Duration,
    exclude: Option<RecallExclude>,
}

impl RecallEachTurn {
    /// Waits at most `timeout` for the current message's pack.
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            exclude: None,
        }
    }

    /// Leaves out a pack whose every cited hit `exclude` matches.
    ///
    /// This is the seam for a host that already shows some memory in the
    /// system prompt. A pack with any hit `exclude` keeps is shown whole:
    /// TinyMemory renders the pack before the host sees it, so single lines
    /// cannot be dropped here.
    #[must_use]
    pub(crate) fn excluding(mut self, exclude: RecallExclude) -> Self {
        self.exclude = Some(exclude);
        self
    }

    /// Whether `exclude` matches every hit `pack` cites. A pack citing
    /// nothing (a refusal notice) is never excluded.
    fn excludes(&self, pack: &TurnPack) -> bool {
        self.exclude.as_ref().is_some_and(|exclude| {
            !pack.native.citations.is_empty()
                && pack.native.citations.iter().all(|hit| exclude(hit))
        })
    }
}

/// Logs this user turn and refreshes its context in the background.
///
/// Without `each_turn`, returns the last completed pack immediately (a cold
/// cache returns `None`). With it, waits at most its deadline for the pack
/// recalled for *this* message, and returns `None` when that pack is late,
/// missing or excluded; the background work still completes and is cached.
/// Omitted recall still logs, but cannot consume another identity's pack, and
/// ignores `each_turn`.
pub(crate) async fn pre_turn(
    ctx: &DispatchCtx,
    config: Arc<Config>,
    identity: ResolvedIdentity,
    thread_id: String,
    turn_index: u32,
    user_text: String,
    in_prompt_from: u32,
    at: chrono::DateTime<chrono::Utc>,
    resumed_after_compaction: bool,
    observed_actor: Option<tinymemory_api::ObservedActor>,
    channel: Option<String>,
    each_turn: Option<RecallEachTurn>,
) -> Option<TurnPack> {
    if !config.memory.conversations.enabled && !identity.recall {
        return None;
    }
    let each_turn = each_turn.filter(|_| identity.recall);
    let cache = cache(ctx);
    let key = Key::with_fingerprint(
        &config,
        memory_config_fingerprint(&config),
        &identity,
        &thread_id,
        false,
    );
    let cached = (identity.recall && each_turn.is_none())
        .then(|| completed(&cache, &key))
        .flatten();
    if each_turn.is_none() {
        tracing::debug!(thread_id = %key.thread, agent_id = %key.identity.agent_id, cache_hit = cached.is_some(), "[memory:prefetch] pre_turn cache");
    }
    let (answer, answered) = match each_turn {
        Some(_) => {
            let (answer, answered) = tokio::sync::oneshot::channel();
            (Some(answer), Some(answered))
        }
        None => (None, None),
    };
    let dispatch = ctx.clone();
    let shown = (config.clone(), identity.clone(), thread_id.clone());
    let work = guard_turn_work(
        &config.clone(),
        Box::pin(dispatch.clone().enter(async move {
            hooks::pre_turn_work(
                &dispatch,
                &config,
                &identity,
                thread_id,
                turn_index,
                user_text,
                in_prompt_from,
                at,
                resumed_after_compaction,
                observed_actor,
                channel,
            )
            .await
        })),
    );
    let work: Work = match answer {
        Some(answer) => Box::pin(async move {
            let pack = work.await;
            // The turn may have stopped waiting; the cache still keeps the pack.
            let _ = answer.send(pack.clone());
            pack
        }),
        None => work,
    };
    let (thread, agent) = (key.thread.clone(), key.identity.agent_id.clone());
    enqueue(cache, key, work);
    let pack = match (each_turn, answered) {
        (Some(each_turn), Some(answered)) => {
            current_pack(&each_turn, answered, &thread, &agent).await
        }
        _ => cached,
    };
    if let Some(pack) = &pack {
        let (config, identity, thread_id) = shown;
        hooks::deliver_citations(ctx, &config, &identity, &thread_id, pack);
    }
    pack
}

/// The pack recalled for the current message, if it answers within the
/// deadline and is not excluded. Logs ids and timings only, never content.
async fn current_pack(
    each_turn: &RecallEachTurn,
    answered: tokio::sync::oneshot::Receiver<Option<TurnPack>>,
    thread: &str,
    agent: &str,
) -> Option<TurnPack> {
    let timeout_ms = u64::try_from(each_turn.timeout.as_millis()).unwrap_or(u64::MAX);
    match tokio::time::timeout(each_turn.timeout, answered).await {
        Ok(Ok(Some(pack))) if each_turn.excludes(&pack) => {
            tracing::debug!(thread_id = %thread, agent_id = %agent, "[memory:recall] every cited hit is excluded; no note");
            None
        }
        Ok(Ok(Some(pack))) => {
            tracing::debug!(thread_id = %thread, agent_id = %agent, refs = pack.refs.len(), "[memory:recall] current message recalled in time");
            Some(pack)
        }
        Ok(Ok(None)) => {
            tracing::debug!(thread_id = %thread, agent_id = %agent, "[memory:recall] nothing recalled for the current message");
            None
        }
        Ok(Err(_)) => {
            tracing::warn!(thread_id = %thread, agent_id = %agent, timeout_ms, "[memory:recall] deadline: recall work ended without an answer (skipped or cancelled); no note");
            None
        }
        Err(_) => {
            tracing::debug!(thread_id = %thread, agent_id = %agent, timeout_ms, "[memory:recall] deadline: recall not ready in time; no note");
            None
        }
    }
}

fn guard_turn_work(config: &Config, work: Work) -> Work {
    let mutation = crate::memory::tool_writes::automatic_mutation(config);
    Box::pin(async move { mutation.run(work).await.flatten() })
}

/// Uses only completed context while queuing recall of the folded span. The
/// authoritative summarizer never waits for this refresh.
pub(crate) fn compaction(
    ctx: &DispatchCtx,
    config: Arc<Config>,
    identity: ResolvedIdentity,
    thread: String,
    dropped: Vec<tinymemory_api::Turn>,
) -> Option<TurnPack> {
    if !identity.recall || dropped.is_empty() {
        return None;
    }
    let cache = cache(ctx);
    let fingerprint = memory_config_fingerprint(&config);
    let key = Key::with_fingerprint(&config, fingerprint, &identity, &thread, true);
    let pack = completed(&cache, &key).or_else(|| {
        completed(
            &cache,
            &Key::with_fingerprint(&config, fingerprint, &identity, &thread, false),
        )
    });
    let dispatch = ctx.clone();
    enqueue(
        cache,
        key,
        Box::pin(ctx.enter(async move {
            hooks::compaction_work(&dispatch, &config, &identity, &thread, dropped).await
        })),
    );
    pack
}

#[cfg(test)]
pub(crate) async fn settle_for_test(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ResolvedIdentity,
    thread: &str,
    compaction: bool,
) {
    let cache = cache(ctx);
    let key = Key::with_fingerprint(
        config,
        memory_config_fingerprint(config),
        identity,
        thread,
        compaction,
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !cache.0.is_idle(&key) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("automatic memory refresh completed");
}

#[cfg(test)]
#[path = "prefetch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prefetch_recall_each_turn_tests.rs"]
mod recall_each_turn_tests;
