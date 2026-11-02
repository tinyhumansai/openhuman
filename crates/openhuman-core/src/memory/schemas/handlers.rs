//! Thin JSON-RPC handlers: parse params, load config, delegate to the ops,
//! and persist config for the setters.

use std::future::Future;

use crate::config::Config;
use crate::core::all::ControllerFuture;
use crate::core::runtime::DispatchCtx;
use crate::core::{Outcome, StructuredRpcError};
use crate::memory::explore::{self, ExploreParams, ItemsGetParams};
use crate::memory::lifecycle::views::{self, JobsRunParams, PackPreviewParams, PolicySetParams};
use crate::memory::scope;
use crate::memory::types::{
    EmptyParams, EngineSetParams, EraseAllParams, FetchParams, ForgetParams, ImportStartParams,
    ImportStateView, ItemsListParams, LearnParams, RecallParams, SourceAddedView,
    SourceRemovedView, SourcesAddParams, SourcesListView, SourcesRemoveParams, SourcesSyncParams,
    SourcesSyncView,
};
use crate::memory::{backfill, brain, engine, import, layout_migration, ops, sources};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tinymemory_api::{Error, Reach, Result};
use tinymemory_tools::confine::{Confinement, Policy};
use tinymemory_tools::layout::identity::Identity;

fn rpc_confinement(config: &Config) -> Confinement {
    let identity = Identity::root().resolve(&scope::identity_pins(config));
    Confinement {
        allowed: Reach::subtree(identity.root().clone()),
        policy: Policy::Refuse,
    }
}

/// The operation run after the common request parsing and config load.
trait RpcOperation<P, R>: Send + 'static {
    type Future: Future<Output = Result<R>> + Send + 'static;

    fn call(self, ctx: DispatchCtx, config: Config, params: P) -> Self::Future;
}

impl<P, R, F, Fut> RpcOperation<P, R> for F
where
    F: FnOnce(DispatchCtx, Config, P) -> Fut + Send + 'static,
    Fut: Future<Output = Result<R>> + Send + 'static,
{
    type Future = Fut;

    fn call(self, ctx: DispatchCtx, config: Config, params: P) -> Self::Future {
        self(ctx, config, params)
    }
}

/// Runs the memory RPC's shared parse → load → call → finish sequence.
fn rpc<P, R>(
    ctx: DispatchCtx,
    params: Map<String, Value>,
    operation: impl RpcOperation<P, R>,
) -> ControllerFuture
where
    P: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
{
    Box::pin(async move {
        let params = parse::<P>(params)?;
        let config = load(&ctx).await?;
        finish(operation.call(ctx, config, params).await)
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> std::result::Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|error| {
        StructuredRpcError::from(Error::InvalidRequest(format!("invalid params: {error}"))).encode()
    })
}

fn to_json<T: Serialize>(value: T) -> std::result::Result<Value, String> {
    Outcome::new(value, Vec::new()).into_cli_compatible_json()
}

fn finish<T: Serialize>(result: Result<T>) -> std::result::Result<Value, String> {
    to_json(result.map_err(|error| StructuredRpcError::from(error).encode())?)
}

async fn load(ctx: &DispatchCtx) -> std::result::Result<Config, String> {
    ctx.load_config().await
}

async fn save(config: &Config) -> Result<()> {
    config
        .save()
        .await
        .map_err(|error| Error::Engine(format!("saving config failed: {error:#}")))
}

fn engine_set_save_error(error: anyhow::Error) -> String {
    if let Some(conflict) = crate::config::ops::config_save_conflict(&error, false) {
        return conflict.encode();
    }
    StructuredRpcError::from(Error::Engine(format!("saving config failed: {error:#}"))).encode()
}

pub(super) fn engines_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(ops::engines_list(&config).await)
    })
}

pub(super) fn engine_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(ops::engine_get(&config).await)
    })
}

pub(super) fn engine_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = parse::<EngineSetParams>(params)?;
        let mut config = load(&ctx).await?;
        if let Err(error) = ops::apply_engine_set(&mut config, &params).await {
            return finish::<Value>(Err(error));
        }
        if let Err(error) = config.save().await {
            return Err(engine_set_save_error(error));
        }
        engine::invalidate();
        finish(Ok(ops::engine_get(&config).await))
    })
}

pub(super) fn recall(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<RecallParams, _>(
        ctx,
        params,
        |_, config, mut params: RecallParams| async move {
            params.filter = Some(rpc_confinement(&config).filter(params.filter)?);
            ops::recall(&config, params).await
        },
    )
}

pub(super) fn fetch(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<FetchParams, _>(
        ctx,
        params,
        |_, config, mut params: FetchParams| async move {
            params.filter = Some(rpc_confinement(&config).filter(params.filter)?);
            ops::fetch(&config, params).await
        },
    )
}

pub(super) fn learn(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<LearnParams, _>(
        ctx,
        params,
        |_, config, mut params: LearnParams| async move {
            let identity = Identity::root().resolve(&scope::identity_pins(&config));
            params.meta = Some(
                rpc_confinement(&config).learn_meta(params.meta, identity.layout.learnings())?,
            );
            ops::learn(&config, params, None).await
        },
    )
}

pub(super) fn forget(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<ForgetParams, _>(
        ctx,
        params,
        |_, config, mut params: ForgetParams| async move {
            params.reach = Some(rpc_confinement(&config).reach(params.reach)?);
            ops::forget(&config, params).await
        },
    )
}

pub(super) fn erase_all(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EraseAllParams, _>(ctx, params, |ctx, config, params| async move {
        ops::erase_all(&ctx, &config, params).await
    })
}

pub(super) fn items_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<ItemsListParams, _>(
        ctx,
        params,
        |_, config, mut params: ItemsListParams| async move {
            let narrowed = explore::narrowed(params.filter.take(), &params.path)?;
            params.filter = Some(rpc_confinement(&config).filter(Some(narrowed))?);
            params.path.clear();
            ops::items_list(&config, params).await
        },
    )
}

pub(super) fn explore(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<ExploreParams, _>(
        ctx,
        params,
        |_, config, mut params: ExploreParams| async move {
            let narrowed = explore::narrowed(params.filter.take(), &params.path)?;
            params.filter = Some(rpc_confinement(&config).filter(Some(narrowed))?);
            params.path.clear();
            explore::explore(&config, params).await
        },
    )
}

pub(super) fn items_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<ItemsGetParams, _>(
        ctx,
        params,
        |_, config, mut params: ItemsGetParams| async move {
            params.reach = Some(rpc_confinement(&config).reach(params.reach)?);
            explore::items_get(&config, params).await
        },
    )
}

pub(super) fn policy_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(views::policy_view(&config))
    })
}

pub(super) fn policy_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<PolicySetParams, _>(ctx, params, |_, mut config, params| async move {
        views::apply_policy_set(&mut config, &params)?;
        save(&config).await?;
        Ok(views::policy_view(&config))
    })
}

pub(super) fn pack_preview(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<PackPreviewParams, _>(ctx, params, |_, config, params| async move {
        views::pack_preview(&config, &Identity::root(), params).await
    })
}

pub(super) fn agents_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        views::agents_list(&config, &Identity::root()).await
    })
}

pub(super) fn conversations_backfill_status(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |ctx, config, _| async move {
        backfill::status(&ctx, &config).await
    })
}

pub(super) fn conversations_backfill_start(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    rpc::<backfill::BackfillStartParams, _>(ctx, params, |ctx, config, params| async move {
        backfill::start(&ctx, &config, params).await
    })
}

pub(super) fn brain_sources(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        brain::sources(&config, &Identity::root()).await
    })
}

pub(super) fn brain_search(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<brain::BrainSearchParams, _>(ctx, params, |_, config, params| async move {
        brain::search(&config, &Identity::root(), params).await
    })
}

pub(super) fn brain_ingest(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<brain::BrainIngestParams, _>(ctx, params, |_, config, params| async move {
        brain::ingest(&config, &Identity::root(), params).await
    })
}

pub(super) fn brain_forget(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<brain::BrainForgetParams, _>(ctx, params, |_, config, params| async move {
        brain::forget(&config, &Identity::root(), params).await
    })
}

pub(super) fn jobs_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(views::jobs_list(&config).await)
    })
}

pub(super) fn jobs_run(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<JobsRunParams, _>(ctx, params, |_, config, params| async move {
        views::jobs_run(&config, params).await
    })
}

pub(super) fn sources_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(SourcesListView {
            sources: sources::list(&config),
        })
    })
}

pub(super) fn sources_add(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<SourcesAddParams, _>(ctx, params, |_, mut config, params| async move {
        let source = sources::apply_add(&mut config, &params)?;
        save(&config).await?;
        Ok(SourceAddedView {
            source: sources::view(&source, None),
        })
    })
}

pub(super) fn sources_remove(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<SourcesRemoveParams, _>(
        ctx,
        params,
        |_, mut config, params: SourcesRemoveParams| async move {
            let Some(removed) = sources::apply_remove(&mut config, &params.id) else {
                return Ok(SourceRemovedView { removed: false });
            };
            save(&config).await?;
            sources::state::remove(&config.workspace_dir, &removed.id);
            if params.forget_items.unwrap_or(false) {
                sources::forget_items(&config, &removed.id).await?;
            }
            Ok(SourceRemovedView { removed: true })
        },
    )
}

pub(super) fn sources_sync(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<SourcesSyncParams, _>(
        ctx,
        params,
        |ctx, config, params: SourcesSyncParams| async move {
            let started = sources::start_sync(&ctx, &config, params.id.as_deref()).await?;
            Ok(SourcesSyncView { started })
        },
    )
}

pub(super) fn import_scan(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |ctx, config, _| async move {
        import::scan(&ctx, &config).await
    })
}

pub(super) fn import_start(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<ImportStartParams, _>(
        ctx,
        params,
        |ctx, config, params: ImportStartParams| async move {
            import::start(&ctx, &config, params.consent)
                .await
                .map(|state| ImportStateView { state })
        },
    )
}

pub(super) fn import_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        Ok(json!(ImportStateView {
            state: import::status(&config)
        }))
    })
}

pub(super) fn import_retry_failed(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |ctx, config, _| async move {
        import::retry_failed(&ctx, &config)
            .await
            .map(|state| ImportStateView { state })
    })
}

pub(super) fn migration_scan(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        layout_migration::scan(&layout_migration::AppHost::new(config)).await
    })
}

pub(super) fn migration_start(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<layout_migration::StartParams, _>(
        ctx,
        params,
        |ctx: DispatchCtx, config: Config, params: layout_migration::StartParams| async move {
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
            layout_migration::status(&config)
        },
    )
}

pub(super) fn migration_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        layout_migration::status(&config)
    })
}

pub(super) fn migration_retry(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    rpc::<EmptyParams, _>(ctx, params, |_, config, _| async move {
        layout_migration::retry(&config)
    })
}

#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;
