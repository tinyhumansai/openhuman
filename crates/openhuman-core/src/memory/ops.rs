//! Engine-facing operations: list and select engines, recall, fetch, learn,
//! forget, erase everything and list items.
//!
//! Every operation takes the [`Config`] it runs against, so the RPC handlers
//! (which load config per call) and the agent tool (which carries its
//! session's config) share one implementation. Every write is
//! scrubbed of secrets and PII by TinyMemory's native engine binder.

#[cfg(feature = "channels")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "channels")]
use std::path::Path;

use chrono::Utc;
#[cfg(feature = "channels")]
use tinymemory_api::ItemKind;
use tinymemory_api::{
    ForgetTarget, MemoryMeta, MetaFilter, Namespace, Reach, RecallMode, StoreItem, StoreReceipt,
    ThreadWindow, TimeHint, WriteOptions,
};
use tinymemory_tools::confine::{Confinement, Policy};
use tinymemory_tools::ops::{FetchOptions, LearningOptions, ListOptions, MemoryOps, RecallOptions};

use crate::config::Config;

use super::engine::{
    self, Binding, BoundEngine, CORTEXDB_ENGINE, DISABLED_ENGINE, TINYHUMANS_ENGINE,
};
use super::types::{
    EngineSetParams, EngineStatus, EngineView, EnginesListView, EraseAllParams, EraseAllView,
    FetchParams, FetchView, ForgetParams, ForgetView, ItemsListParams, ItemsListView, LearnParams,
    LearnView, RecallParams, RecallView, RefersTo,
};
use tinymemory_api::{Error, Result};

/// Apply the SaaS tenant boundary to a host memory operation. The namespace
/// mechanics live in TinyMemory; this adapter only selects the authenticated
/// profile root and preserves the existing fail-closed error.
pub(crate) fn saas_confinement(config: &Config, policy: Policy) -> Result<Option<Confinement>> {
    if !crate::core::runtime::is_saas() {
        return Ok(None);
    }
    let root = config
        .memory
        .root
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .parse::<Namespace>();
    match root {
        Ok(root) if root != Namespace::ROOT => Ok(Some(Confinement {
            allowed: Reach::subtree(root),
            policy,
        })),
        Ok(_) | Err(_) => {
            tracing::error!("[memory:scope] no valid confinement root in SaaS mode");
            Err(Error::InvalidRequest(
                "memory is unavailable: this agent has no valid memory root".into(),
            ))
        }
    }
}

/// Default confidence of a learning stored without one.
pub const DEFAULT_LEARNING_CONFIDENCE: f32 = tinymemory_tools::ops::DEFAULT_LEARNING_CONFIDENCE;

/// `memory_engines_list`.
#[must_use]
pub async fn engines_list(config: &Config) -> EnginesListView {
    let active = match engine::resolve(config).await {
        Binding::On(bound) => Some(bound.id),
        Binding::Off { .. } => None,
    };
    EnginesListView {
        engines: tinymemory_integrations::list_engines(),
        active,
    }
}

/// `memory_engine_get`: the configured engine, its credential and health.
pub async fn engine_get(config: &Config) -> EngineView {
    let configured = Some(config.memory.engine.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    match engine::resolve(config).await {
        Binding::On(bound) => {
            let health = bound.engine.health().await;
            let (status, reason) = match health {
                tinymemory_api::EngineHealth::Ok => (EngineStatus::Ok, None),
                tinymemory_api::EngineHealth::Degraded(reason) => {
                    (EngineStatus::Degraded, Some(reason))
                }
                tinymemory_api::EngineHealth::Down(reason) => (EngineStatus::Down, Some(reason)),
            };
            tracing::debug!(engine = %bound.id, ?status, "[memory:ops] engine_get");
            EngineView {
                engine: Some(bound.id.clone()),
                endpoint: Some(bound.endpoint.clone()),
                has_key: engine::has_key(config, &bound.id).await,
                status,
                reason,
                fetch_modes: bound.engine.descriptor().fetch_modes.clone(),
            }
        }
        Binding::Off {
            engine: id,
            endpoint,
            reason,
        } => {
            let id = id.or(configured);
            EngineView {
                has_key: match id.as_deref() {
                    Some(id) => engine::has_key(config, id).await,
                    None => false,
                },
                engine: id,
                endpoint,
                status: EngineStatus::Off,
                reason: Some(reason),
                fetch_modes: Vec::new(),
            }
        }
    }
}

/// Applies `memory_engine_set` to `config` in memory (and to the credential
/// store for a key). The caller persists `config`.
pub async fn apply_engine_set(config: &mut Config, params: &EngineSetParams) -> Result<()> {
    let engine_id = params.engine.trim();
    if engine_id == DISABLED_ENGINE {
        // Turning memory off keeps every engine's endpoint and key, so
        // switching back needs no re-entry.
        if params.endpoint.is_some() || params.api_key.is_some() {
            return Err(Error::InvalidRequest(
                ("disabling memory takes no endpoint or API key").into(),
            ));
        }
        config.memory.engine = DISABLED_ENGINE.to_string();
        tracing::info!("[memory:ops] memory disabled");
        return Ok(());
    }
    if !tinymemory_integrations::list_engines()
        .iter()
        .any(|descriptor| descriptor.id == engine_id)
    {
        return Err(Error::InvalidRequest(format!(
            "unknown memory engine `{engine_id}`"
        )));
    }
    if let Some(endpoint) = params.endpoint.as_deref() {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            config.memory.engines.remove(engine_id);
        } else {
            validate_endpoint(endpoint)?;
            config
                .memory
                .engines
                .entry(engine_id.to_string())
                .or_default()
                .endpoint = Some(endpoint.to_string());
        }
    }
    if let Some(key) = params.api_key.as_deref() {
        match engine_id {
            CORTEXDB_ENGINE if key.trim().is_empty() => {
                engine::clear_cortexdb_key(config).await?;
            }
            CORTEXDB_ENGINE => engine::store_cortexdb_key(config, key).await?,
            TINYHUMANS_ENGINE => {
                return Err(Error::InvalidRequest(
                    ("the tinyhumans engine uses your sign-in, not an API key").into(),
                ));
            }
            _ => {
                return Err(Error::InvalidRequest(
                    ("this engine takes no API key").into(),
                ));
            }
        }
    }
    config.memory.engine = engine_id.to_string();
    tracing::info!(engine = %engine_id, "[memory:ops] engine selected");
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|_| Error::InvalidRequest("the endpoint is not a valid URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::InvalidRequest(
            ("the endpoint must be an http(s) URL with a host").into(),
        ));
    }
    Ok(())
}

async fn bound(config: &Config) -> Result<BoundEngine> {
    engine::resolve(config).await.engine()
}

/// The native request adapter with the authenticated SaaS profile boundary,
/// or no namespace boundary for a single-user host.
pub(crate) fn memory_ops(config: &Config) -> Result<MemoryOps> {
    Ok(match saas_confinement(config, Policy::Clamp)? {
        Some(confinement) => MemoryOps::new(confinement),
        None => MemoryOps::unconfined(),
    })
}

/// The [`TimeHint`] for `refers_to`, in the user's time zone
/// ([`Config::time_zone`]).
fn time_hint(config: &Config, refers_to: Option<RefersTo>) -> Result<Option<TimeHint>> {
    let Some(RefersTo { from, to }) = refers_to else {
        return Ok(None);
    };
    TimeHint::new(from, to.unwrap_or(from), Some(config.time_zone())).map(Some)
}

/// `memory_recall`.
pub async fn recall(config: &Config, params: RecallParams) -> Result<RecallView> {
    recall_with(config, params, RecallMode::Answer, None).await
}

/// [`recall`] in `mode`, leaving `exclude_thread`'s turns out. The agent's
/// memory tool asks for [`RecallMode::Evidence`] (its own model answers) and
/// excludes the thread it runs in, whose user turn is logged before the tool
/// reads and would otherwise come back as a memory.
pub async fn recall_with(
    config: &Config,
    params: RecallParams,
    mode: RecallMode,
    exclude_thread: Option<ThreadWindow>,
) -> Result<RecallView> {
    let bound = bound(config).await?;
    let request = memory_ops(config)?.recall_request(RecallOptions {
        question: params.question,
        filter: params.filter,
        limit: params.limit,
        instructions: None,
        refers_to: time_hint(config, params.refers_to)?,
        exclude_thread,
        mode,
    })?;
    let answer = bound.engine.recall(request).await?;
    tracing::debug!(
        engine = %bound.id,
        citations = answer.citations.len(),
        ?mode,
        "[memory:ops] recall answered"
    );
    Ok(RecallView {
        answer: answer.answer,
        citations: answer.citations,
        model: answer.model,
    })
}

/// `memory_fetch`. An unset mode uses the engine's first declared mode; a
/// mode the engine does not declare is `UNSUPPORTED`.
pub async fn fetch(config: &Config, params: FetchParams) -> Result<FetchView> {
    fetch_with(config, params, None).await
}

/// [`fetch`] leaving `exclude_thread`'s turns out (see [`recall_with`]).
pub async fn fetch_with(
    config: &Config,
    params: FetchParams,
    exclude_thread: Option<ThreadWindow>,
) -> Result<FetchView> {
    let bound = bound(config).await?;
    let descriptor = bound.engine.descriptor();
    match params.mode {
        Some(mode) => descriptor.ensure_mode(mode)?,
        None if descriptor.fetch_modes.is_empty() => {
            return Err(Error::Unsupported("the engine offers no fetch mode".into()));
        }
        None => {}
    }
    let ops = memory_ops(config)?;
    let refers_to = time_hint(config, params.refers_to)?;
    let request = ops.fetch_request(
        FetchOptions {
            query: params.query,
            mode: params.mode,
            filter: params.filter,
            limit: params.limit,
            cursor: params.cursor,
            refers_to,
            exclude_thread,
        },
        descriptor,
    )?;
    let page = bound.engine.fetch(request).await?;
    tracing::debug!(engine = %bound.id, hits = page.hits.len(), "[memory:ops] fetch answered");
    Ok(FetchView {
        hits: page.hits,
        next_cursor: page.next_cursor,
    })
}

/// Builds the learning `memory_learn` (and the `memory` tool's `learn`)
/// stores. `host_meta` wins over the caller's `meta` field by field, so a
/// caller cannot overwrite what the host knows (thread, agent, tool call).
pub fn learning_item(params: LearnParams, host_meta: Option<MemoryMeta>) -> Result<StoreItem> {
    MemoryOps::unconfined().learning_item(LearningOptions {
        text: params.text,
        kind: params.kind,
        confidence: params.confidence,
        meta: params.meta,
        host_meta,
        default_namespace: Namespace::ROOT,
        now: Utc::now(),
    })
}

/// `memory_learn`: returns once the learning is readable.
pub async fn learn(
    config: &Config,
    params: LearnParams,
    host_meta: Option<MemoryMeta>,
) -> Result<LearnView> {
    learn_with(config, params, host_meta, WriteOptions::visible()).await
}

/// [`learn`], returning as soon as `options` allows. The agent's `memory`
/// tool passes [`WriteOptions::accepted`] so a turn never waits on the
/// engine indexing the learning.
pub async fn learn_with(
    config: &Config,
    params: LearnParams,
    host_meta: Option<MemoryMeta>,
    options: WriteOptions,
) -> Result<LearnView> {
    let item = learning_item(params, host_meta)?;
    let item = memory_ops(config)?.store_item(item)?;
    let bound = bound(config).await?;
    let receipt = bound.engine.store_with(item, options).await?;
    super::lifecycle::prefetch::invalidate(config);
    Ok(LearnView { id: receipt.id.0 })
}

/// Scrubs `item` and stores it on the bound engine.
pub async fn store_item(config: &Config, item: StoreItem) -> Result<StoreReceipt> {
    store_item_with(config, item, WriteOptions::visible()).await
}

/// [`store_item`], returning as soon as `options` allows: a host capturing
/// as it goes passes [`WriteOptions::accepted`] so it never waits on the
/// engine indexing the item.
pub async fn store_item_with(
    config: &Config,
    item: StoreItem,
    options: WriteOptions,
) -> Result<StoreReceipt> {
    let ops = memory_ops(config)?;
    let bound = bound(config).await?;
    let item = ops.store_item(item)?;
    let receipt = bound.engine.store_with(item, options).await?;
    super::lifecycle::prefetch::invalidate(config);
    Ok(receipt)
}

/// `memory_forget`.
pub async fn forget(config: &Config, params: ForgetParams) -> Result<ForgetView> {
    if !params.ids.iter().any(|id| !id.trim().is_empty()) {
        return Err(Error::InvalidRequest(
            ("forget needs at least one id").into(),
        ));
    }
    let bound = bound(config).await?;
    let report = memory_ops(config)?
        .forget(bound.engine.as_ref(), params.ids, params.reach)
        .await?;
    tracing::debug!(engine = %bound.id, forgotten = report.forgotten, "[memory:ops] forget");
    super::lifecycle::prefetch::invalidate(config);
    Ok(ForgetView {
        forgotten: report.forgotten,
    })
}

/// Forgets every item matching a non-empty metadata filter through the bound
/// engine, applying the host-selected confinement.
///
/// # Errors
///
/// Returns an error if memory is unavailable, the filter is invalid, or the
/// engine cannot complete the deletion.
pub async fn forget_filter(config: &Config, filter: MetaFilter) -> Result<usize> {
    let bound = bound(config).await?;
    let report = memory_ops(config)?
        .forget_filter(bound.engine.as_ref(), filter)
        .await?;
    tracing::debug!(
        engine = %bound.id,
        forgotten = report.forgotten,
        "[memory:ops] filtered forget"
    );
    super::lifecycle::prefetch::invalidate(config);
    Ok(report.forgotten)
}

/// Forgets conversation memory owned by an external channel on disconnect.
///
/// The engine filter removes stamped turns in one operation. A bounded
/// snapshot of canonical thread IDs plus the previous release's read-only
/// channel map supplies thread-level fallback for turns that predate channel
/// metadata. New turns never write that map; remove the compatibility reader
/// after one release.
#[cfg(feature = "channels")]
pub async fn forget_channel(config: &Config, channel: &str) -> Result<usize> {
    let channel = channel.trim().to_ascii_lowercase();
    if channel.is_empty() {
        return Ok(0);
    }
    let legacy_threads = legacy_channel_threads(&config.workspace_dir, &channel);
    let bound = match engine::resolve(config).await.engine() {
        Ok(bound) => Some(bound),
        Err(Error::Off(_)) => None,
        Err(error) => return Err(error),
    };

    // Capture the request's tenant/profile before the module call and the
    // thread-store wrappers move work off the caller's task.
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let identity = crate::modules::channel_controller::prepare_identity(
        &crate::modules::client(config.clone()),
        tinychannels_bus::module_inbound::PrepareChannelIdentityRequest {
            channel: channel.clone(),
            sender: None,
            reply_target: None,
            thread_ts: None,
        },
    )
    .await;
    let base_thread_id = match identity {
        Ok(identity) => identity.thread_id,
        Err(error) => {
            enqueue_channel_threads(config, &legacy_threads);
            return Err(Error::Engine(format!(
                "channel identity preparation failed: {error}"
            )));
        }
    };
    let threads =
        channel_thread_snapshot(&ctx, config, &channel, &base_thread_id, legacy_threads).await?;
    let Some(bound) = bound else {
        enqueue_channel_threads(config, &threads);
        tracing::debug!(channel = %channel, queued = threads.len(), "[memory:channels] channel forget queued");
        return Ok(0);
    };

    let filter = MetaFilter {
        channel: Some(channel.clone()),
        ..MetaFilter::kinds([ItemKind::Conversation])
    };
    let mut forgotten = match bound.engine.forget(ForgetTarget::Filter(filter)).await {
        Ok(report) => report.forgotten,
        Err(Error::Off(_)) => {
            enqueue_channel_threads(config, &threads);
            return Ok(0);
        }
        Err(error) => {
            enqueue_channel_threads(config, &threads);
            return Err(error);
        }
    };

    let mut failure = None;
    for thread_id in &threads {
        match forget_channel_thread(&bound, thread_id).await {
            Ok(count) => forgotten += count,
            Err(error) => {
                super::deletion::enqueue(
                    &config.workspace_dir,
                    super::deletion::PendingDeletion::Thread {
                        thread_id: thread_id.clone(),
                    },
                );
                if !matches!(error, Error::Off(_)) && failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }

    tracing::debug!(
        channel = %channel,
        forgotten,
        queued = super::deletion::pending(&config.workspace_dir).len(),
        "[memory:channels] channel forgotten"
    );
    match failure {
        Some(error) => Err(error),
        None => Ok(forgotten),
    }
}

#[cfg(feature = "channels")]
fn channel_key(channel: &str) -> String {
    channel.trim().to_ascii_lowercase()
}

/// Reads only the legacy map written by prior releases. Corrupt JSON is skipped
/// without moving the source; current code never creates or updates it.
#[cfg(feature = "channels")]
fn legacy_channel_threads(workspace: &Path, channel: &str) -> BTreeSet<String> {
    // Prefer the workspace state value, which may have imported and removed
    // the old file. If no value is available (including a failed import),
    // retain the legacy read path. Malformed JSON is skipped without moving
    // or rewriting its source.
    if let Ok(Some(all)) = super::state_store::load_strict::<BTreeMap<String, BTreeSet<String>>>(
        workspace,
        "channel_threads",
    ) {
        return all.get(&channel_key(channel)).cloned().unwrap_or_default();
    }
    let file = workspace.join("memory").join("channel_threads.json");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return BTreeSet::new();
    };
    let all: BTreeMap<String, BTreeSet<String>> =
        serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(%error, "[memory:channels] legacy channel map unparsable; skipped");
            BTreeMap::new()
        });
    all.get(&channel_key(channel)).cloned().unwrap_or_default()
}

#[cfg(feature = "channels")]
fn is_channel_thread_id(thread_id: &str, base_thread_id: &str) -> bool {
    if thread_id == base_thread_id {
        return true;
    }
    thread_id
        .strip_prefix(base_thread_id)
        .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with("#thread:"))
}

#[cfg(feature = "channels")]
fn enqueue_channel_threads(config: &Config, threads: &BTreeSet<String>) {
    for thread_id in threads {
        super::deletion::enqueue(
            &config.workspace_dir,
            super::deletion::PendingDeletion::Thread {
                thread_id: thread_id.clone(),
            },
        );
    }
}

#[cfg(feature = "channels")]
async fn channel_thread_snapshot(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    channel: &str,
    base_thread_id: &str,
    mut owned: BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let threads = match crate::threads::store::blocking::list_threads(
        ctx.clone(),
        config.workspace_dir.clone(),
    )
    .await
    {
        Ok(threads) => threads,
        Err(error) => {
            enqueue_channel_threads(config, &owned);
            return Err(Error::Engine(format!(
                "reading channel thread snapshot failed: {error}"
            )));
        }
    };
    for thread in threads {
        if owned.contains(&thread.id) || is_channel_thread_id(&thread.id, base_thread_id) {
            owned.insert(thread.id);
            continue;
        }
        let messages = match crate::threads::store::blocking::get_messages(
            ctx.clone(),
            config.workspace_dir.clone(),
            thread.id.clone(),
        )
        .await
        {
            Ok(messages) => messages,
            Err(error) => {
                enqueue_channel_threads(config, &owned);
                return Err(Error::Engine(format!(
                    "reading channel message snapshot failed: {error}"
                )));
            }
        };
        if messages.iter().any(|message| {
            message
                .extra_metadata
                .get("channel")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| channel_key(value) == channel)
        }) {
            owned.insert(thread.id);
        }
    }
    Ok(owned)
}

#[cfg(feature = "channels")]
async fn forget_channel_thread(bound: &BoundEngine, thread_id: &str) -> Result<usize> {
    let filter = MetaFilter {
        thread_id: Some(thread_id.to_string()),
        ..MetaFilter::kinds([ItemKind::Conversation])
    };
    Ok(bound
        .engine
        .forget(ForgetTarget::Filter(filter))
        .await?
        .forgotten)
}

/// `memory_erase_all`: erases everything the bound engine holds, the whole
/// tree, every kind. Needs `confirm: true`.
///
/// On the hosted engine this is one `DELETE /memory`, which erases the
/// caller's entire hosted memory (every scope under their tenant, the
/// pre-v3 layout's included). On CortexDB reached directly it erases every
/// kind scope of the bound layout (the person's `org:<id>` subtree under
/// layout v3, and the retired `user:<id>` one while it is still read); a legacy tree is left alone, because on a self-hosted CortexDB
/// other accounts may share it (see `layout_migration`). An engine that
/// cannot erase answers `UNSUPPORTED`.
pub async fn erase_all(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    params: EraseAllParams,
) -> Result<EraseAllView> {
    if !params.confirm {
        return Err(Error::InvalidRequest(
            ("erasing all memory needs confirm: true; nothing erased comes back").into(),
        ));
    }
    let bound = bound(config).await?;
    // Earlier durable tool writes must finish or be discarded before the
    // remote wipe; hold the owner barrier until the erase has completed.
    let _writes = super::tool_writes::fence_and_clear(ctx, config).await?;
    super::lifecycle::prefetch::invalidate(config);
    let request = MemoryOps::unconfined().erase_all_request(true)?;
    tracing::info!(engine = %bound.id, "[memory:ops] erase_all: erasing the whole memory");
    let report = bound.engine.erase(request).await.map_err(|error| {
        tracing::warn!(engine = %bound.id, error = %error, "[memory:ops] erase_all failed");
        error
    })?;
    tracing::info!(
        engine = %bound.id,
        erased_scopes = report.erased_scopes,
        "[memory:ops] erase_all: done"
    );
    super::lifecycle::prefetch::invalidate(config);
    Ok(EraseAllView {
        erased_scopes: report.erased_scopes,
    })
}

/// `memory_items_list`.
pub async fn items_list(config: &Config, params: ItemsListParams) -> Result<ItemsListView> {
    let bound = bound(config).await?;
    let preview = params.preview;
    let filter = super::explore::narrowed(params.filter, &params.path)?;
    let request = memory_ops(config)?.list_request(ListOptions {
        filter: Some(filter),
        path: Vec::new(),
        limit: params.limit,
        cursor: params.cursor,
    })?;
    let page = if preview {
        bound.engine.list_preview(request).await?
    } else {
        bound.engine.list(request).await?
    };
    Ok(ItemsListView {
        items: page.items,
        next_cursor: page.next_cursor,
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_saas_tests.rs"]
mod saas_tests;
