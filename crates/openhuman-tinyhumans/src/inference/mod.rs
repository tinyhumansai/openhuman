//! The managed inference provider: chat model, model catalog and cloud embeddings.
//!
//! Plugs into the core through the generic registries on
//! `openhuman_embed::__host::inference_registry`, keyed by the managed provider
//! id ([`PROVIDER_OPENHUMAN`]), so existing configs resolve identically:
//!
//! | Registry | Registered value |
//! | --- | --- |
//! | chat models | [`managed_model::OpenHumanBackendModel`] built per request, plus its readiness probe |
//! | embeddings | [`managed_embedding::ManagedEmbeddingModel`] |
//! | catalogs | the managed `/openai/v1/models?catalog=openrouter` listing and its limits request |
//!
//! `register` is the one place it does so; `crate::register::register_all`
//! calls it from `install()`.

mod managed_calls;
mod managed_catalog;
mod managed_embedding;
mod managed_model;
mod managed_reasoning;
mod managed_session;
mod managed_usage;

use std::sync::Arc;

use openhuman_embed::__host::config::{Config, MODEL_MANAGED_DEFAULT};
use openhuman_embed::__host::inference_registry::{
    register_catalog, register_chat_model, register_embedding, register_inspection_chat_model,
    register_inspection_embedding, BoxFuture, ChatModelEntry, ChatModelFactory, ChatModelRequest,
    InspectionChatModelBuild, ReadinessProbe, PROVIDER_OPENHUMAN,
};
use tinyinference_llm::model::ChatModel;

pub use managed_embedding::ManagedEmbeddingModel;
pub use managed_model::{ManagedOptions, OpenHumanBackendModel};

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
    request: &ChatModelRequest<'_>,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let pinned = request
        .model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(MODEL_MANAGED_DEFAULT)
        .to_string();
    let default_model = request
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pinned.clone());
    let options = managed_options(request.config);
    log::debug!(
        "[tinyhumans][inference] building managed chat model role={} model={} native_tools={} threaded={}",
        request.role,
        default_model,
        request.native_tool_calling,
        request.thread_id.is_some()
    );
    let model =
        OpenHumanBackendModel::new(request.config.api_url.as_deref(), &options, default_model)
            .with_native_tool_calling(request.native_tool_calling)
            .with_thread_id(request.thread_id);
    Ok((Arc::new(model), pinned))
}

/// Builds a managed model around a frozen read-only credential snapshot for
/// inspection calls; invoke and stream never consult mutable auth state.
fn build_chat_model_for_inspection(
    request: &ChatModelRequest<'_>,
) -> anyhow::Result<InspectionChatModelBuild> {
    let pinned = request
        .model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(MODEL_MANAGED_DEFAULT)
        .to_string();
    let default_model = request
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pinned.clone());
    let options = managed_options(request.config);
    let bearer = managed_session::managed_bearer_for_inspection(request.config)?;
    let endpoint = format!(
        "{}/openai/v1",
        crate::backend::url::effective_api_url(&request.config.api_url).trim_end_matches('/')
    );
    let model = OpenHumanBackendModel::new(
        request.config.api_url.as_deref(),
        &options,
        default_model.clone(),
    )
    .with_native_tool_calling(request.native_tool_calling)
    .with_thread_id(request.thread_id)
    .with_inspection_bearer(bearer.clone());
    Ok(InspectionChatModelBuild {
        model: Arc::new(model),
        model_id: default_model,
        endpoint,
        credential: bearer,
    })
}

fn readiness_probe() -> ReadinessProbe {
    Arc::new(
        |role: String, model: String, config: Config| -> BoxFuture<Result<(), String>> {
            Box::pin(async move {
                log::debug!("[tinyhumans][inference] readiness probe role={role} model={model}");
                let options = managed_options(&config);
                OpenHumanBackendModel::new(config.api_url.as_deref(), &options, model)
                    .probe_readiness()
                    .await
            })
        },
    )
}

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    let build: ChatModelFactory = Arc::new(build_chat_model);
    register_chat_model(
        PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build,
            probe: Some(readiness_probe()),
        },
    );
    register_inspection_chat_model(
        PROVIDER_OPENHUMAN,
        Arc::new(build_chat_model_for_inspection),
    );
    register_embedding(PROVIDER_OPENHUMAN, |request| {
        Ok(Box::new(ManagedEmbeddingModel::from_request(request)))
    });
    register_inspection_embedding(PROVIDER_OPENHUMAN, |request| {
        ManagedEmbeddingModel::from_request_for_inspection(request)
            .map(|model| Box::new(model) as Box<dyn tinyinference_embeddings::EmbeddingModel>)
    });
    register_catalog(PROVIDER_OPENHUMAN, managed_catalog::entry());
    openhuman_embed::__host::inference_registry::register_inspection_catalog(
        PROVIDER_OPENHUMAN,
        managed_catalog::inspection_entry(),
    );
    log::debug!(
        "[tinyhumans] inference: registered the managed chat model, embeddings and catalog"
    );
}
