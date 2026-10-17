//! Thin JSON-RPC handlers: parse params, load config, delegate to the ops,
//! and persist config for the setters.

use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::config::Config;
use crate::core::all::ControllerFuture;
use crate::core::Outcome;
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::explore::{ExploreParams, ItemsGetParams};
use crate::memory::lifecycle::views::{self, JobsRunParams, PackPreviewParams, PolicySetParams};
use crate::memory::scope::MemoryIdentity;
use crate::memory::types::{
    EmptyParams, EngineSetParams, EraseAllParams, FetchParams, ForgetParams, ImportStartParams,
    ImportStateView, ItemsListParams, LearnParams, RecallParams, SourceAddedView,
    SourceRemovedView, SourcesAddParams, SourcesListView, SourcesRemoveParams, SourcesSyncParams,
    SourcesSyncView,
};
use crate::memory::{backfill, brain, confine, engine, import, layout_migration, ops, sources};

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

async fn load(ctx: &DispatchCtx) -> Result<Config, String> {
    ctx.load_config().await
}

async fn save(config: &Config) -> MemoryResult<()> {
    config
        .save()
        .await
        .map_err(|error| MemoryError::Engine(format!("saving config failed: {error:#}")))
}

pub(super) fn engines_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(ops::engines_list(&load(&ctx).await?))
    })
}

pub(super) fn engine_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(ops::engine_get(&load(&ctx).await?).await)
    })
}

pub(super) fn engine_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<EngineSetParams>(params)?;
        let mut config = load(&ctx).await?;
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

pub(super) fn recall(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<RecallParams>(params)?;
        finish(confine::recall(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn fetch(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<FetchParams>(params)?;
        finish(confine::fetch(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn learn(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<LearnParams>(params)?;
        finish(confine::learn(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn forget(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ForgetParams>(params)?;
        finish(confine::forget(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn erase_all(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<EraseAllParams>(params)?;
        finish(ops::erase_all(&ctx, &load(&ctx).await?, params).await)
    })
}

pub(super) fn items_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ItemsListParams>(params)?;
        finish(confine::items_list(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn explore(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ExploreParams>(params)?;
        finish(confine::explore(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn items_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ItemsGetParams>(params)?;
        finish(confine::items_get(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn policy_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(views::policy_view(&load(&ctx).await?))
    })
}

pub(super) fn policy_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<PolicySetParams>(params)?;
        let mut config = load(&ctx).await?;
        views::apply_policy_set(&mut config, &params).map_err(String::from)?;
        save(&config).await.map_err(String::from)?;
        to_json(views::policy_view(&config))
    })
}

pub(super) fn pack_preview(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<PackPreviewParams>(params)?;
        finish(views::pack_preview(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn agents_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(views::agents_list(&load(&ctx).await?, &MemoryIdentity::root()).await)
    })
}

pub(super) fn conversations_backfill_status(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(backfill::status(&ctx, &load(&ctx).await?).await)
    })
}

pub(super) fn conversations_backfill_start(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<backfill::BackfillStartParams>(params)?;
        finish(backfill::start(&ctx, &load(&ctx).await?, params).await)
    })
}

pub(super) fn brain_sources(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(brain::sources(&load(&ctx).await?, &MemoryIdentity::root()).await)
    })
}

pub(super) fn brain_search(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<brain::BrainSearchParams>(params)?;
        finish(brain::search(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn brain_ingest(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<brain::BrainIngestParams>(params)?;
        finish(brain::ingest(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn brain_forget(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<brain::BrainForgetParams>(params)?;
        finish(brain::forget(&load(&ctx).await?, &MemoryIdentity::root(), params).await)
    })
}

pub(super) fn jobs_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(views::jobs_list(&load(&ctx).await?).await)
    })
}

pub(super) fn jobs_run(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<JobsRunParams>(params)?;
        finish(views::jobs_run(&load(&ctx).await?, params).await)
    })
}

pub(super) fn sources_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        to_json(SourcesListView {
            sources: sources::list(&load(&ctx).await?),
        })
    })
}

pub(super) fn sources_add(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesAddParams>(params)?;
        let mut config = load(&ctx).await?;
        let source = sources::apply_add(&mut config, &params).map_err(String::from)?;
        save(&config).await.map_err(String::from)?;
        to_json(SourceAddedView {
            source: sources::view(&source, None),
        })
    })
}

pub(super) fn sources_remove(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesRemoveParams>(params)?;
        let mut config = load(&ctx).await?;
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

pub(super) fn sources_sync(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<SourcesSyncParams>(params)?;
        let config = load(&ctx).await?;
        let started =
            sources::start_sync(&ctx, &config, params.id.as_deref()).map_err(String::from)?;
        to_json(SourcesSyncView { started })
    })
}

pub(super) fn import_scan(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(import::scan(&load(&ctx).await?).await)
    })
}

pub(super) fn import_start(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<ImportStartParams>(params)?;
        let config = load(&ctx).await?;
        finish(
            import::start(&ctx, &config, params.consent)
                .await
                .map(|state| ImportStateView { state }),
        )
    })
}

pub(super) fn import_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        let config = load(&ctx).await?;
        to_json(json!(ImportStateView {
            state: import::status(&config)
        }))
    })
}

pub(super) fn import_retry_failed(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        let config = load(&ctx).await?;
        finish(
            import::retry_failed(&ctx, &config)
                .await
                .map(|state| ImportStateView { state }),
        )
    })
}

pub(super) fn migration_scan(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(layout_migration::scan(&layout_migration::AppHost::new(load(&ctx).await?)).await)
    })
}

pub(super) fn migration_start(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<layout_migration::StartParams>(params)?;
        let config = load(&ctx).await?;
        let started = layout_migration::start(
            &ctx,
            config.clone(),
            std::sync::Arc::new(layout_migration::AppHost::new(config.clone())),
            layout_migration::Trigger::Manual {
                takeover: params.takeover,
            },
            std::sync::Arc::new(import::scheduler_paused),
        );
        tracing::debug!(started, "[memory:layout_migration] start requested");
        finish(layout_migration::status(&config))
    })
}

pub(super) fn migration_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(layout_migration::status(&load(&ctx).await?))
    })
}

pub(super) fn migration_retry(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        parse::<EmptyParams>(params)?;
        finish(layout_migration::retry(&load(&ctx).await?))
    })
}
