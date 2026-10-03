//! Thin JSON-RPC handlers: parse params, load config, delegate to the ops,
//! and persist config for the setters.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::config::Config;
use crate::core::all::ControllerFuture;
use crate::core::Outcome;
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::types::{
    ContextSetParams, ConversationsSetParams, EmptyParams, EngineSetParams, FetchParams,
    ForgetParams, ImportStartParams, ImportStateView, ItemsListParams, LearnParams, RecallParams,
    SourceAddedView, SourceRemovedView, SourcesAddParams, SourcesListView, SourcesRemoveParams,
    SourcesSyncParams, SourcesSyncView,
};
use crate::memory::{context, conversations, engine, import, ops, sources};

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params))
        .map_err(|error| MemoryError::invalid(format!("invalid params: {error}")).into())
}

fn to_json<T: Serialize>(value: T) -> Result<Value, String> {
    Outcome::new(value, Vec::new()).into_cli_compatible_json()
}

fn finish<T: Serialize>(result: MemoryResult<T>) -> Result<Value, String> {
    to_json(result.map_err(String::from)?)
}

async fn load() -> Result<Config, String> {
    crate::config::rpc::load_config_with_timeout().await
}

async fn save(config: &Config) -> MemoryResult<()> {
    config
        .save()
        .await
        .map_err(|error| MemoryError::Engine(format!("saving config failed: {error:#}")))
}

pub(super) fn engines_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(ops::engines_list(&load().await?))
    })
}

pub(super) fn engine_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(ops::engine_get(&load().await?).await)
    })
}

pub(super) fn engine_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<EngineSetParams>(params)?;
        let mut config = load().await?;
        let result = async {
            ops::apply_engine_set(&mut config, &params)?;
            save(&config).await
        }
        .await;
        result.map_err(String::from)?;
        engine::invalidate();
        to_json(ops::engine_get(&config).await)
    })
}

pub(super) fn recall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<RecallParams>(params)?;
        finish(ops::recall(&load().await?, params).await)
    })
}

pub(super) fn fetch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<FetchParams>(params)?;
        finish(ops::fetch(&load().await?, params).await)
    })
}

pub(super) fn learn(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<LearnParams>(params)?;
        finish(ops::learn(&load().await?, params, None).await)
    })
}

pub(super) fn forget(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ForgetParams>(params)?;
        finish(ops::forget(&load().await?, params).await)
    })
}

pub(super) fn items_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ItemsListParams>(params)?;
        finish(ops::items_list(&load().await?, params).await)
    })
}

pub(super) fn conversations_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(conversations::view(&load().await?))
    })
}

pub(super) fn conversations_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ConversationsSetParams>(params)?;
        let mut config = load().await?;
        let was_enabled = config.memory.conversations.enabled;
        conversations::apply_set(&mut config, &params).map_err(String::from)?;
        save(&config).await.map_err(String::from)?;
        if was_enabled && !config.memory.conversations.enabled {
            conversations::flush_all(&config).await;
        }
        to_json(conversations::view(&config))
    })
}

pub(super) fn sources_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(SourcesListView {
            sources: sources::list(&load().await?),
        })
    })
}

pub(super) fn sources_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesAddParams>(params)?;
        let mut config = load().await?;
        let source = sources::apply_add(&mut config, &params).map_err(String::from)?;
        save(&config).await.map_err(String::from)?;
        to_json(SourceAddedView {
            source: sources::view(&source, None),
        })
    })
}

pub(super) fn sources_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesRemoveParams>(params)?;
        let mut config = load().await?;
        let Some(removed) = sources::apply_remove(&mut config, &params.id) else {
            return to_json(SourceRemovedView { removed: false });
        };
        save(&config).await.map_err(String::from)?;
        sources::state::remove(&config.workspace_dir, &removed.id);
        if params.forget_items.unwrap_or(false) {
            sources::forget_items(&config, &removed.id)
                .await
                .map_err(String::from)?;
        }
        to_json(SourceRemovedView { removed: true })
    })
}

pub(super) fn sources_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesSyncParams>(params)?;
        let config = load().await?;
        let started = sources::start_sync(&config, params.id.as_deref()).map_err(String::from)?;
        to_json(SourcesSyncView { started })
    })
}

pub(super) fn context_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(context::view(&load().await?))
    })
}

pub(super) fn context_refresh(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(context::refresh(&load().await?).await)
    })
}

pub(super) fn context_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ContextSetParams>(params)?;
        let mut config = load().await?;
        context::apply_set(&mut config, &params).map_err(String::from)?;
        save(&config).await.map_err(String::from)?;
        if let Err(error) = crate::cron::system_jobs::ensure_memory_jobs(&config) {
            tracing::warn!(error = %error, "[memory:rpc] rescheduling memory jobs failed");
        }
        to_json(context::view(&config))
    })
}

pub(super) fn import_scan(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(import::scan(&load().await?).await)
    })
}

pub(super) fn import_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ImportStartParams>(params)?;
        let config = load().await?;
        finish(
            import::start(&config, params.consent)
                .await
                .map(|state| ImportStateView { state }),
        )
    })
}

pub(super) fn import_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        let config = load().await?;
        to_json(json!(ImportStateView {
            state: import::status(&config)
        }))
    })
}
