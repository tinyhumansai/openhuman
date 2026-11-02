//! The managed inference provider: chat model, model catalog and cloud embeddings.
//!
//! Plugs into the core through the generic registries on
//! `openhuman_embed::__host::inference_registry`, keyed by the managed provider
//! id ([`PROVIDER_OPENHUMAN`]), so existing configs resolve identically:
//!
//! | Registry | Registered value |
//! | --- | --- |
//! | chat models | TinyInference's [`tinyinference_llm::providers::tinyhumans::TinyHumansModel`] with host credentials, plus its readiness probe |
//! | embeddings | TinyInference's `CloudEmbeddingModel` with OpenHuman's credential and egress policy |
//! | catalogs | the managed `/openai/v1/models?catalog=openrouter` listing and its limits request |
//!
//! `register` is the one place it does so; `crate::register::register_all`
//! calls it from `install()`.

mod backend_error_code;
mod managed_catalog;
mod managed_embedding;
mod managed_model;
mod managed_session;
mod tiers;
#[cfg(test)]
pub(crate) use tiers::MODEL_MANAGED_DEFAULT;

use std::sync::Arc;

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::inference_registry::{
    register_backend_error_classifier, register_catalog, register_chat_model,
    register_config_normalizer, register_embedding, register_inspection_catalog,
    register_inspection_chat_model, register_inspection_embedding, ChatModelEntry,
    ChatModelFactory, ChatModelRequest, EmbeddingRequest, InspectionChatModelBuild,
    ProviderContext, ProviderModelMetadata, ReadinessProbe, PROVIDER_OPENHUMAN,
};
use tinyinference_llm::model::ChatModel;

pub use managed_model::ManagedOptions;

use crate::install::InstallOptions;

fn managed_options(config: &Config) -> ManagedOptions {
    // The config's own workspace directory, so the model's credential store is
    // the one login wrote to (not an unrelated `~/.openhuman`).
    ManagedOptions {
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        auth_profile_override: None,
    }
}

/// Builds the managed chat model for one registry request. The core has already
/// resolved the concrete model id (`model_override`) and disclosed the egress.
pub(crate) fn build_chat_model(
    request: &ChatModelRequest<'_, ProviderContext>,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let pinned = request
        .model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .or_else(tiers::default_model)
        .ok_or_else(|| anyhow::anyhow!("TinyHumans managed inference has no configured model"))?;
    let default_model = request
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pinned.clone());
    let options = managed_options(&request.context.config);
    log::debug!(
        "[tinyhumans][inference] building managed chat model role={} model={} native_tools={} threaded={}",
        request.role,
        default_model,
        request.native_tool_calling,
        request.thread_id.is_some()
    );
    let model = managed_model::build_chat_model(
        request.context.config.api_url.as_deref(),
        &options,
        &default_model,
        request.native_tool_calling,
        request.thread_id,
        true,
    )?;
    Ok((Arc::new(model), pinned))
}

async fn build_chat_model_for_inspection(
    request: &ChatModelRequest<'_, ProviderContext>,
) -> anyhow::Result<InspectionChatModelBuild> {
    let config = &request.context.config;
    let pinned = request
        .model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .or_else(tiers::default_model)
        .ok_or_else(|| anyhow::anyhow!("TinyHumans managed inference has no configured model"))?;
    let default_model = request
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pinned.clone());
    let bearer = managed_session::managed_bearer_for_inspection(config).await?;
    let model = managed_model::build_chat_model_for_inspection(
        config.api_url.as_deref(),
        &default_model,
        bearer.clone(),
        request.native_tool_calling,
        request.thread_id,
    )?;
    let endpoint = format!(
        "{}/openai/v1",
        crate::backend::url::effective_api_url(&config.api_url).trim_end_matches('/')
    );
    Ok(InspectionChatModelBuild {
        model: Arc::new(model),
        model_id: default_model,
        endpoint,
        credential: bearer,
    })
}

fn readiness_probe() -> ReadinessProbe<ProviderContext> {
    Arc::new(
        |request: tinyinference_llm::providers::overrides::ReadinessRequest<ProviderContext>| -> tinyinference_llm::providers::overrides::BoxFuture<Result<(), String>> {
            Box::pin(async move {
                let role = request.role;
                let model = request.model;
                let config = request.context.config;
                log::debug!("[tinyhumans][inference] readiness probe role={role} model={model}");
                let options = managed_options(&config);
                let managed = match managed_model::build_chat_model(
                    config.api_url.as_deref(),
                    &options,
                    &model,
                    true,
                    None,
                    false,
                ) {
                    Ok(managed) => managed,
                    Err(error) => {
                        log::debug!("[tinyhumans][inference] readiness model construction failed, failing open: {error}");
                        return Ok(());
                    }
                };
                managed.probe_readiness().await
            })
        },
    )
}

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    let build: ChatModelFactory<ProviderContext> = Arc::new(|request| {
        build_chat_model(request)
            .map_err(|error| tinyinference_llm::Error::Model(error.to_string()))
    });
    register_chat_model(
        PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build,
            probe: Some(readiness_probe()),
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(tiers::default_model),
                role_for_model: Arc::new(tiers::role_for_model),
                is_alias: Arc::new(tiers::is_alias),
                is_raw_model: Arc::new(tiers::is_raw_model),
                supports_vision: Arc::new(tiers::supports_vision),
                resolve_model: Arc::new(|role, context, model| {
                    tiers::resolve_model(role, &context.config, model)
                }),
                context_window: Arc::new(tiers::context_window),
                default_pricing: Arc::new(tiers::default_pricing),
                without_reasoning: Arc::new(tinyinference_llm::model::without_reasoning),
            }),
        },
    );
    register_embedding(
        PROVIDER_OPENHUMAN,
        Arc::new(|request: EmbeddingRequest<ProviderContext>| {
            Box::pin(async move {
                Ok(Box::new(managed_embedding::from_request(request))
                    as Box<dyn tinyinference_embeddings::EmbeddingModel>)
            })
        }),
    );
    register_inspection_chat_model(
        PROVIDER_OPENHUMAN,
        Arc::new(|request| Box::pin(build_chat_model_for_inspection(request))),
    );
    register_inspection_embedding(PROVIDER_OPENHUMAN, |request| {
        Box::pin(async move {
            managed_embedding::from_request_for_inspection(&request)
                .await
                .map(|model| Box::new(model) as Box<dyn tinyinference_embeddings::EmbeddingModel>)
        })
    });
    register_catalog(PROVIDER_OPENHUMAN, managed_catalog::entry());
    register_inspection_catalog(PROVIDER_OPENHUMAN, managed_catalog::inspection_entry());
    register_config_normalizer(PROVIDER_OPENHUMAN, Arc::new(tiers::normalize_config));
    register_backend_error_classifier(Arc::new(backend_error_code::classify));
    log::debug!(
        "[tinyhumans] inference: registered the managed chat model, embeddings and catalog"
    );
}
