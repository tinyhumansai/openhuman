//! Best-effort, scoped automatic memory refresh. Turns only read completed
//! packs; all engine work and date extraction run in bounded background workers.
//! Packs live in memory for one minute and are never persisted by this adapter.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use tinymemory_tools::prefetch::{Limits, PrefetchRuntime, Prefetcher, WorkerPool};

use crate::config::Config;
use crate::core::runtime::{slot_for, DispatchCtx};
use crate::memory::scope::ResolvedIdentity;

use super::hooks::{self, PreTurnInput, TurnPack};

const MAX_PACK_BYTES: usize = 64 * 1024;
static WORKERS: LazyLock<WorkerPool> = LazyLock::new(|| WorkerPool::new(8));
static CACHES: LazyLock<Mutex<Vec<Weak<Cache>>>> = LazyLock::new(Mutex::default);

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

fn pack_fits(pack: &TurnPack) -> bool {
    serde_json::to_vec(pack)
        .ok()
        .zip(serde_json::to_vec(&pack.citations).ok())
        .is_some_and(|(pack, citations)| {
            pack.len().saturating_add(citations.len()) <= MAX_PACK_BYTES
        })
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
    fn new(config: &Config, identity: &ResolvedIdentity, thread: &str, compaction: bool) -> Self {
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        // Keep only the digest, never serialized settings or credentials.
        serde_json::to_value(config)
            .ok()
            .and_then(|value| serde_json::to_vec(&value).ok())
            .unwrap_or_default()
            .hash(&mut fingerprint);
        config.config_path.hash(&mut fingerprint);
        config.action_dir.hash(&mut fingerprint);
        Self {
            workspace: config.workspace_dir.clone(),
            config: fingerprint.finish(),
            identity: identity.clone(),
            thread: thread.to_owned(),
            compaction,
        }
    }
}

struct Cache(Prefetcher<Key, TurnPack>);

impl Default for Cache {
    fn default() -> Self {
        Self(
            Prefetcher::new(Arc::new(HostRuntime))
                .with_limits(Limits::default())
                .with_workers(WORKERS.clone())
                .with_filter(pack_fits),
        )
    }
}

fn cache(ctx: &DispatchCtx) -> Arc<Cache> {
    let cache = slot_for::<Cache>(ctx);
    let mut caches = CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    caches.retain(|known| known.strong_count() > 0);
    if !caches
        .iter()
        .any(|known| known.ptr_eq(&Arc::downgrade(&cache)))
    {
        caches.push(Arc::downgrade(&cache));
    }
    cache
}

/// Discards completed packs for this workspace, across its agent contexts.
/// Pending conversation logs are preserved; results of reads already in flight
/// cannot republish after invalidation. Call after
/// successful memory writes and erasure; no engine work runs here.
pub(crate) fn invalidate(config: &Config) {
    let mut caches = CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    caches.retain(|known| {
        let Some(cache) = known.upgrade() else {
            return false;
        };
        cache
            .0
            .invalidate_where(|key| key.workspace == config.workspace_dir);
        true
    });
}

/// Clears all automatic packs after engine or credential invalidation. Old
/// in-flight completions cannot recreate entries they no longer own.
pub(crate) fn invalidate_all() {
    let mut caches = CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    caches.retain(|known| {
        let Some(cache) = known.upgrade() else {
            return false;
        };
        cache.0.invalidate_all();
        true
    });
}

fn completed(cache: &Cache, key: &Key) -> Option<TurnPack> {
    cache.0.completed(key)
}

fn enqueue(cache: Arc<Cache>, key: Key, work: Work) {
    tracing::debug!(thread_id = %key.thread, agent_id = %key.identity.agent_id, compaction = key.compaction, "[memory:prefetch] enqueue refresh");
    cache.0.enqueue(key, work);
}

/// Returns the last completed pack immediately, then logs this user turn and
/// refreshes its context in the background. A cold cache returns `None`.
/// Omitted recall still logs, but cannot consume another identity's pack.
pub(crate) fn pre_turn(
    ctx: &DispatchCtx,
    config: Arc<Config>,
    identity: ResolvedIdentity,
    input: PreTurnInput,
    channel: Option<String>,
) -> Option<TurnPack> {
    if !config.memory.conversations.enabled && !identity.recall {
        return None;
    }
    let cache = cache(ctx);
    let key = Key::new(&config, &identity, &input.thread_id, false);
    let pack = identity.recall.then(|| completed(&cache, &key)).flatten();
    tracing::debug!(thread_id = %key.thread, agent_id = %key.identity.agent_id, cache_hit = pack.is_some(), "[memory:prefetch] pre_turn cache");
    if let Some(pack) = &pack {
        crate::memory::tools::record_pack_citations(ctx, &input.thread_id, pack.citations.clone());
    }
    let dispatch = ctx.clone();
    enqueue(
        cache,
        key,
        guard_turn_work(
            &config.clone(),
            Box::pin(dispatch.clone().enter(async move {
                if config.memory.conversations.enabled {
                    if let Some(channel) = channel {
                        let workspace = config.workspace_dir.clone();
                        let thread = input.thread_id.clone();
                        let _ = dispatch
                            .spawn_blocking(move || {
                                crate::memory::channels::record(&workspace, &channel, &thread)
                            })
                            .await;
                    }
                }
                hooks::pre_turn_work(&dispatch, &config, &identity, input).await
            })),
        ),
    );
    pack.map(cached_notice)
}

fn guard_turn_work(config: &Config, work: Work) -> Work {
    let mutation = crate::memory::tool_writes::automatic_mutation(config);
    Box::pin(async move { mutation.run(work).await.flatten() })
}

fn cached_notice(mut pack: TurnPack) -> TurnPack {
    let notice = "This is cached memory from a completed background lookup for an earlier turn. It may be incomplete or unrelated to the current question; use explicit memory recall or fetch when you need a current answer.\n\n";
    pack.markdown.insert_str(0, notice);
    pack.tokens += tinymemory_tools::recall::estimate_tokens(notice);
    pack
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
    let key = Key::new(&config, &identity, &thread, true);
    let pack = completed(&cache, &key)
        .or_else(|| completed(&cache, &Key::new(&config, &identity, &thread, false)));
    enqueue(
        cache,
        key,
        Box::pin(ctx.enter(async move {
            hooks::compaction_work(&config, &identity, &thread, dropped).await
        })),
    );
    pack.map(cached_notice)
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
    let key = Key::new(config, identity, thread, compaction);
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
