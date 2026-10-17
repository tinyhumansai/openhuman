//! Engine-facing operations: list and select engines, recall, fetch, learn,
//! forget, erase everything and list items.
//!
//! Every operation takes the [`Config`] it runs against, so the RPC handlers
//! (which load config per call) and the agent tool (which carries its
//! session's config) share one implementation. Every write is
//! scrubbed of secrets and PII by the bound engine itself ([`super::guard`]).

use chrono::Utc;
use tinymemory_api::{
    EraseRequest, FetchRequest, ForgetTarget, ItemId, LearningKind, ListRequest, MemoryMeta,
    MetaFilter, RecallMode, RecallRequest, StoreItem, StoreReceipt, ThreadWindow, TimeHint,
    WriteOptions,
};

use crate::config::Config;

use super::engine::{
    self, Binding, BoundEngine, CORTEXDB_ENGINE, DISABLED_ENGINE, TINYHUMANS_ENGINE,
};
use super::error::{MemoryError, MemoryResult};
use super::types::{
    clamp_limit, EngineSetParams, EngineStatus, EngineView, EnginesListView, EraseAllParams,
    EraseAllView, FetchParams, FetchView, ForgetParams, ForgetView, ItemsListParams, ItemsListView,
    LearnParams, LearnView, RecallParams, RecallView, RefersTo,
};

/// Default confidence of a learning stored without one.
pub const DEFAULT_LEARNING_CONFIDENCE: f32 = 0.8;

/// `memory_engines_list`.
#[must_use]
pub fn engines_list(config: &Config) -> EnginesListView {
    let active = match engine::resolve(config) {
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
    match engine::resolve(config) {
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
                has_key: engine::has_key(config, &bound.id),
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
                has_key: id.as_deref().is_some_and(|id| engine::has_key(config, id)),
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
pub fn apply_engine_set(config: &mut Config, params: &EngineSetParams) -> MemoryResult<()> {
    let engine_id = params.engine.trim();
    if engine_id == DISABLED_ENGINE {
        // Turning memory off keeps every engine's endpoint and key, so
        // switching back needs no re-entry.
        if params.endpoint.is_some() || params.api_key.is_some() {
            return Err(MemoryError::invalid(
                "disabling memory takes no endpoint or API key",
            ));
        }
        config.memory.engine = DISABLED_ENGINE.to_string();
        engine::invalidate();
        tracing::info!("[memory:ops] memory disabled");
        return Ok(());
    }
    if !tinymemory_integrations::list_engines()
        .iter()
        .any(|descriptor| descriptor.id == engine_id)
    {
        return Err(MemoryError::invalid(format!(
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
                engine::clear_cortexdb_key(config)?;
            }
            CORTEXDB_ENGINE => engine::store_cortexdb_key(config, key)?,
            TINYHUMANS_ENGINE => {
                return Err(MemoryError::invalid(
                    "the tinyhumans engine uses your sign-in, not an API key",
                ))
            }
            _ => return Err(MemoryError::invalid("this engine takes no API key")),
        }
    }
    config.memory.engine = engine_id.to_string();
    engine::invalidate();
    tracing::info!(engine = %engine_id, "[memory:ops] engine selected");
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> MemoryResult<()> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|_| MemoryError::invalid("the endpoint is not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(MemoryError::invalid(
            "the endpoint must be an http(s) URL with a host",
        ));
    }
    Ok(())
}

fn bound(config: &Config) -> MemoryResult<BoundEngine> {
    engine::resolve(config).engine()
}

/// The [`TimeHint`] for `refers_to`, in the user's time zone
/// ([`Config::time_zone`]).
fn time_hint(config: &Config, refers_to: Option<RefersTo>) -> MemoryResult<Option<TimeHint>> {
    let Some(RefersTo { from, to }) = refers_to else {
        return Ok(None);
    };
    TimeHint::new(from, to.unwrap_or(from), Some(config.time_zone()))
        .map(Some)
        .map_err(MemoryError::from)
}

/// `memory_recall`.
pub async fn recall(config: &Config, params: RecallParams) -> MemoryResult<RecallView> {
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
) -> MemoryResult<RecallView> {
    let bound = bound(config)?;
    let request = RecallRequest {
        question: params.question,
        filter: confine_filter(config, params.filter.unwrap_or_default())?,
        limit: clamp_limit(params.limit),
        instructions: None,
        refers_to: time_hint(config, params.refers_to)?,
        exclude_thread,
        mode,
    };
    request.validate()?;
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
pub async fn fetch(config: &Config, params: FetchParams) -> MemoryResult<FetchView> {
    fetch_with(config, params, None).await
}

/// [`fetch`] leaving `exclude_thread`'s turns out (see [`recall_with`]).
pub async fn fetch_with(
    config: &Config,
    params: FetchParams,
    exclude_thread: Option<ThreadWindow>,
) -> MemoryResult<FetchView> {
    let bound = bound(config)?;
    let descriptor = bound.engine.descriptor();
    let mode = match params.mode {
        Some(mode) => {
            descriptor.ensure_mode(mode)?;
            mode
        }
        None => *descriptor
            .fetch_modes
            .first()
            .ok_or_else(|| MemoryError::Unsupported("the engine offers no fetch mode".into()))?,
    };
    let request = FetchRequest {
        query: params.query,
        mode,
        filter: confine_filter(config, params.filter.unwrap_or_default())?,
        limit: clamp_limit(params.limit),
        cursor: params.cursor,
        beliefs: 0,
        max_scopes: None,
        refers_to: time_hint(config, params.refers_to)?,
        exclude_thread,
    };
    request.validate()?;
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
pub fn learning_item(
    params: LearnParams,
    host_meta: Option<MemoryMeta>,
) -> MemoryResult<StoreItem> {
    let confidence = params.confidence.unwrap_or(DEFAULT_LEARNING_CONFIDENCE);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(MemoryError::invalid("confidence must be between 0 and 1"));
    }
    let mut meta = params.meta.unwrap_or_default();
    if let Some(host) = host_meta {
        merge_meta(&mut meta, host);
    }
    if meta.observed_at.is_none() {
        meta.observed_at = Some(Utc::now());
    }
    let item = StoreItem::learning(
        params.text.trim(),
        params.kind.unwrap_or(LearningKind::Fact),
        confidence,
        meta,
    );
    item.validate()?;
    Ok(item)
}

/// Overlays every field `host` sets onto `meta`.
fn merge_meta(meta: &mut MemoryMeta, host: MemoryMeta) {
    macro_rules! take {
        ($($field:ident),*) => {$(
            if host.$field.is_some() {
                meta.$field = host.$field;
            }
        )*};
    }
    take!(
        workspace,
        folder,
        file_path,
        language,
        repo,
        commit,
        url,
        thread_id,
        turns,
        agent_id,
        tool_call,
        observed_at,
        derive
    );
    meta.source = host.source;
    // The host's node is authoritative: a caller cannot write into another
    // agent's memory.
    meta.namespace = host.namespace;
    for tag in host.tags {
        if !meta.tags.contains(&tag) {
            meta.tags.push(tag);
        }
    }
}

/// `memory_learn`: returns once the learning is readable.
pub async fn learn(
    config: &Config,
    params: LearnParams,
    host_meta: Option<MemoryMeta>,
) -> MemoryResult<LearnView> {
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
) -> MemoryResult<LearnView> {
    let mut item = learning_item(params, host_meta)?;
    // The same confinement as `store_item`: a SaaS user's learning lands in
    // their own tree whatever namespace it names.
    if let Some(root) = super::user_scope::confinement(config)? {
        super::user_scope::clamp_item(&mut item, &root);
    }
    let bound = bound(config)?;
    let receipt = store_on_with(&bound, item, options).await?;
    Ok(LearnView { id: receipt.id.0 })
}

/// Scrubs `item` and stores it on the bound engine.
pub async fn store_item(config: &Config, item: StoreItem) -> MemoryResult<StoreReceipt> {
    store_item_with(config, item, WriteOptions::visible()).await
}

/// [`store_item`], returning as soon as `options` allows: a host capturing
/// as it goes passes [`WriteOptions::accepted`] so it never waits on the
/// engine indexing the item.
pub async fn store_item_with(
    config: &Config,
    mut item: StoreItem,
    options: WriteOptions,
) -> MemoryResult<StoreReceipt> {
    if let Some(root) = super::user_scope::confinement(config)? {
        super::user_scope::clamp_item(&mut item, &root);
    }
    let bound = bound(config)?;
    store_on_with(&bound, item, options).await
}

/// Stores `item` on `bound`; the bound engine scrubs it ([`super::guard`]).
pub async fn store_on(bound: &BoundEngine, item: StoreItem) -> MemoryResult<StoreReceipt> {
    store_on_with(bound, item, WriteOptions::visible()).await
}

/// [`store_on`], returning as soon as `options` allows.
pub async fn store_on_with(
    bound: &BoundEngine,
    item: StoreItem,
    options: WriteOptions,
) -> MemoryResult<StoreReceipt> {
    let kind = item.kind();
    let receipt = bound.engine.store_with(item, options).await?;
    tracing::debug!(
        engine = %bound.id,
        kind = kind.as_str(),
        wait = ?options.wait,
        replayed = receipt.replayed,
        "[memory:ops] item stored"
    );
    Ok(receipt)
}

/// Stores `items` on `bound` in one bulk call, returning as soon as
/// `options` allows. With [`WriteOptions::visible`] each item is listed on
/// return, and ranked recall may lag behind for all but the last. A backfill
/// or an import that reads nothing back until it ends passes
/// [`WriteOptions::accepted`]: an engine that supports it (CortexDB) returns
/// once the items are durably accepted, before they are indexed, and any
/// other stores them as with `visible`.
pub async fn store_many_on_with(
    bound: &BoundEngine,
    items: Vec<StoreItem>,
    options: WriteOptions,
) -> MemoryResult<Vec<StoreReceipt>> {
    let count = items.len();
    let receipts = bound.engine.store_many_with(items, options).await?;
    tracing::debug!(
        engine = %bound.id,
        count,
        wait = ?options.wait,
        replayed = receipts.iter().filter(|r| r.replayed).count(),
        "[memory:ops] batch stored"
    );
    Ok(receipts)
}

/// `memory_forget`.
pub async fn forget(config: &Config, params: ForgetParams) -> MemoryResult<ForgetView> {
    let ids: Vec<ItemId> = params
        .ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .map(ItemId)
        .collect();
    if ids.is_empty() {
        return Err(MemoryError::invalid("forget needs at least one id"));
    }
    let bound = bound(config)?;
    // A SaaS user forgets only what lies in their own tree.
    let reach = match super::user_scope::confinement(config)? {
        Some(root) => Some(super::user_scope::clamp_reach(params.reach, &root)),
        None => params.reach,
    };
    let report = match reach {
        // With a reach the engine forgets by id inside it: `forget_within`
        // never looks beyond the reach for an id, where `ForgetTarget::Ids`
        // sweeps every scope of the tree to find one.
        Some(reach) => {
            let ids = within_reach(&bound, ids, reach.clone()).await?;
            if ids.is_empty() {
                tracing::debug!(engine = %bound.id, "[memory:ops] forget: nothing in reach");
                return Ok(ForgetView { forgotten: 0 });
            }
            bound.engine.forget_within(ids, reach).await?
        }
        None => bound.engine.forget(ForgetTarget::Ids(ids)).await?,
    };
    tracing::debug!(engine = %bound.id, forgotten = report.forgotten, "[memory:ops] forget");
    super::lifecycle::prefetch::invalidate(config);
    Ok(ForgetView {
        forgotten: report.forgotten,
    })
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
) -> MemoryResult<EraseAllView> {
    if !params.confirm {
        return Err(MemoryError::invalid(
            "erasing all memory needs confirm: true; nothing erased comes back",
        ));
    }
    let bound = bound(config)?;
    // Earlier durable tool writes must finish or be discarded before the
    // remote wipe; hold the owner barrier until the erase has completed.
    let _writes = super::tool_writes::fence_and_clear(ctx, config).await?;
    super::lifecycle::prefetch::invalidate(config);
    let mut request = EraseRequest::new(tinymemory_api::Reach::subtree(
        tinymemory_api::Namespace::ROOT,
    ));
    request.whole_tree = true;
    tracing::info!(engine = %bound.id, "[memory:ops] erase_all: erasing the whole memory");
    let report = bound.engine.erase(request).await.map_err(|error| {
        tracing::warn!(engine = %bound.id, error = %error, "[memory:ops] erase_all failed");
        MemoryError::from(error)
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

/// The ids among `ids` naming an item in `reach`, read through `get` in
/// chunks of its id limit.
async fn within_reach(
    bound: &BoundEngine,
    ids: Vec<ItemId>,
    reach: tinymemory_api::Reach,
) -> MemoryResult<Vec<ItemId>> {
    let mut kept = Vec::new();
    for chunk in ids.chunks(tinymemory_api::explore::MAX_GET_IDS) {
        let found = bound
            .engine
            .get(tinymemory_api::GetRequest {
                ids: chunk.to_vec(),
                reach: Some(reach.clone()),
            })
            .await?;
        kept.extend(found.into_iter().map(|hit| hit.id));
    }
    Ok(kept)
}

/// `memory_items_list`.
pub async fn items_list(config: &Config, params: ItemsListParams) -> MemoryResult<ItemsListView> {
    let bound = bound(config)?;
    let request = ListRequest {
        filter: confine_filter(
            config,
            super::explore::narrowed(params.filter, &params.path)?,
        )?,
        limit: clamp_limit(params.limit),
        cursor: params.cursor,
    };
    request.validate()?;
    let page = if params.preview {
        bound.engine.list_preview(request).await?
    } else {
        bound.engine.list(request).await?
    };
    Ok(ItemsListView {
        items: page.items,
        next_cursor: page.next_cursor,
    })
}

/// `filter`, confined to the SaaS user's tree when `config` has one
/// ([`super::user_scope`]).
pub(crate) fn confine_filter(config: &Config, filter: MetaFilter) -> MemoryResult<MetaFilter> {
    Ok(match super::user_scope::confinement(config)? {
        Some(root) => super::user_scope::clamp_filter(filter, &root),
        None => filter,
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
