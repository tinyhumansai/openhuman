//! Engine-facing operations: list and select engines, recall, fetch, learn,
//! forget and list items.
//!
//! Every operation takes the [`Config`] it runs against, so the RPC handlers
//! (which load config per call) and the agent tool (which carries its
//! session's config) share one implementation. Writes go through
//! [`store_item`], which scrubs secrets and PII before anything leaves the
//! process.

use chrono::Utc;
use tinymemory::{
    FetchRequest, ForgetTarget, ItemId, LearningKind, ListRequest, MemoryMeta, RecallRequest,
    StoreItem, StoreReceipt,
};

use crate::config::Config;

use super::engine::{self, Binding, BoundEngine, CORTEXDB_ENGINE, TINYHUMANS_ENGINE};
use super::error::{MemoryError, MemoryResult};
use super::types::{
    clamp_limit, EngineSetParams, EngineStatus, EngineView, EnginesListView, FetchParams,
    FetchView, ForgetParams, ForgetView, ItemsListParams, ItemsListView, LearnParams, LearnView,
    RecallParams, RecallView,
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
        engines: tinymemory::list_engines(),
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
                tinymemory::EngineHealth::Ok => (EngineStatus::Ok, None),
                tinymemory::EngineHealth::Degraded(reason) => {
                    (EngineStatus::Degraded, Some(reason))
                }
                tinymemory::EngineHealth::Down(reason) => (EngineStatus::Down, Some(reason)),
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
    if !tinymemory::list_engines()
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

/// `memory_recall`.
pub async fn recall(config: &Config, params: RecallParams) -> MemoryResult<RecallView> {
    let bound = bound(config)?;
    let request = RecallRequest {
        question: params.question,
        filter: params.filter.unwrap_or_default(),
        limit: clamp_limit(params.limit),
        instructions: None,
    };
    request.validate()?;
    let answer = bound.engine.recall(request).await?;
    tracing::debug!(
        engine = %bound.id,
        citations = answer.citations.len(),
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
        filter: params.filter.unwrap_or_default(),
        limit: clamp_limit(params.limit),
        cursor: params.cursor,
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
        observed_at
    );
    meta.source = host.source;
    for tag in host.tags {
        if !meta.tags.contains(&tag) {
            meta.tags.push(tag);
        }
    }
}

/// `memory_learn`.
pub async fn learn(
    config: &Config,
    params: LearnParams,
    host_meta: Option<MemoryMeta>,
) -> MemoryResult<LearnView> {
    let item = learning_item(params, host_meta)?;
    let receipt = store_item(config, item).await?;
    Ok(LearnView { id: receipt.id.0 })
}

/// Scrubs `item` and stores it on the bound engine.
pub async fn store_item(config: &Config, item: StoreItem) -> MemoryResult<StoreReceipt> {
    let bound = bound(config)?;
    store_on(&bound, item).await
}

/// Scrubs `item` and stores it on `bound`.
pub async fn store_on(bound: &BoundEngine, item: StoreItem) -> MemoryResult<StoreReceipt> {
    let kind = item.kind();
    let scrubbed = tinymemory::safety::scrub_item_with(item, crate::security::scrub::host_policy());
    if scrubbed.report.changed() {
        tracing::debug!(
            kind = kind.as_str(),
            "[memory:ops] item scrubbed before store"
        );
    }
    let receipt = bound.engine.store(scrubbed.value).await?;
    tracing::debug!(
        engine = %bound.id,
        kind = kind.as_str(),
        replayed = receipt.replayed,
        "[memory:ops] item stored"
    );
    Ok(receipt)
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
    let report = bound.engine.forget(ForgetTarget::Ids(ids)).await?;
    tracing::debug!(engine = %bound.id, forgotten = report.forgotten, "[memory:ops] forget");
    Ok(ForgetView {
        forgotten: report.forgotten,
    })
}

/// `memory_items_list`.
pub async fn items_list(config: &Config, params: ItemsListParams) -> MemoryResult<ItemsListView> {
    let bound = bound(config)?;
    let request = ListRequest {
        filter: params.filter.unwrap_or_default(),
        limit: clamp_limit(params.limit),
        cursor: params.cursor,
    };
    request.validate()?;
    let page = bound.engine.list(request).await?;
    Ok(ItemsListView {
        items: page.items,
        next_cursor: page.next_cursor,
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
