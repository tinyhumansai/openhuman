//! The managed backend as a crate-native `ChatModel`: tier pinning per
//! workload role, `hint:*` translation, the privacy gate and the single managed
//! egress emission. The model itself is built by the chat-model provider
//! registered under [`PROVIDER_OPENHUMAN`]
//! ([`crate::inference::registry::register_chat_model`]); with none registered
//! the managed provider is unavailable and construction answers with the
//! `BACKEND_UNAVAILABLE:` sentinel.

use super::*;

type ManagedChatModel = (
    std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    String,
);

/// The error the managed provider answers with when no chat-model provider is
/// registered for it.
pub(crate) fn managed_unavailable_error() -> anyhow::Error {
    anyhow::anyhow!(
        "{} managed inference needs a hosted provider; none is registered",
        crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
    )
}

/// Build the managed model for `role` through the registered chat-model
/// provider. The privacy gate runs first (before a client is constructed or any
/// egress announced), then the model id is resolved and the egress disclosed
/// once: managed resolution is the universal chokepoint for every managed
/// inference construction (privacy epic S2, #4436).
pub(super) fn managed_chat_model(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
    default_model: Option<&str>,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<ManagedChatModel> {
    let extensions = crate::core::runtime::CoreContext::current_extensions();
    managed_chat_model_for(
        extensions.as_ref(),
        role,
        config,
        model_override,
        default_model,
        native_tool_calling,
        thread_id,
        egress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn managed_chat_model_for(
    extensions: Option<&crate::core::slots::Extensions>,
    role: &str,
    config: &Config,
    model_override: Option<&str>,
    default_model: Option<&str>,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<ManagedChatModel> {
    enforce_local_only_inference(role, PROVIDER_OPENHUMAN)?;
    let overrides = crate::inference::registry::provider_overrides(extensions.cloned());
    let Some(entry) = overrides.chat_entry(PROVIDER_OPENHUMAN) else {
        log::debug!("[providers][chat-factory] role={role} managed provider is not registered");
        return Err(managed_unavailable_error());
    };
    let model = match entry.models.as_ref() {
        Some(models) => (models.resolve_model)(
            role,
            &crate::inference::registry::ProviderContext::new(config, egress),
            model_override,
        ),
        None => model_override
            .or(config.default_model.as_deref())
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.starts_with("hint:"))
            .map(str::to_string),
    }
    .ok_or_else(managed_unavailable_error)?;
    log::debug!(
        "[providers][chat-factory] role={role} managed provider served by a registered chat model model={model}"
    );
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference("openhuman", &model, true),
    );
    let context = crate::inference::registry::ProviderContext::new(config, egress);
    let request = tinyinference_llm::providers::overrides::ChatModelRequest {
        role,
        model_override: Some(&model),
        default_model,
        native_tool_calling,
        thread_id,
        context: &context,
    };
    Ok((entry.build)(&request)?)
}

/// Build a managed model only through the host's explicitly inspection-safe
/// callback. This path never falls back to the runtime factory: doing so could
/// defer mutable credential access until invoke/stream.
pub(super) async fn managed_chat_model_for_inspection(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
) -> anyhow::Result<ManagedChatModel> {
    let build = managed_chat_model_build_for_inspection(None, role, config, model_override).await?;
    Ok((build.model, build.model_id))
}

pub(super) async fn managed_chat_model_build_for_inspection(
    extensions: Option<&crate::core::slots::Extensions>,
    role: &str,
    config: &Config,
    model_override: Option<&str>,
) -> anyhow::Result<crate::inference::registry::InspectionChatModelBuild> {
    enforce_local_only_inference(role, PROVIDER_OPENHUMAN)?;
    let Some(factory) =
        crate::inference::registry::inspection_chat_model_for(extensions, PROVIDER_OPENHUMAN)
    else {
        return Err(managed_unavailable_error());
    };
    let overrides = crate::inference::registry::provider_overrides(extensions.cloned());
    let Some(entry) = overrides.chat_entry(PROVIDER_OPENHUMAN) else {
        return Err(managed_unavailable_error());
    };
    let context = crate::inference::registry::ProviderContext::new(config, &EgressLedger::none());
    let model = match entry.models.as_ref() {
        Some(models) => (models.resolve_model)(role, &context, model_override),
        None => model_override
            .or(config.default_model.as_deref())
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.starts_with("hint:"))
            .map(str::to_string),
    }
    .ok_or_else(managed_unavailable_error)?;
    let context = crate::inference::registry::ProviderContext::new(config, &EgressLedger::none());
    let request = tinyinference_llm::providers::overrides::ChatModelRequest {
        role,
        model_override: Some(&model),
        default_model: None,
        native_tool_calling: true,
        thread_id: None,
        context: &context,
    };
    (factory)(&request).await
}

/// Construct a managed turn model with the explicit OpenHuman thread attached.
#[cfg(test)]
pub(crate) fn make_openhuman_backend_model_for_thread(
    role: &str,
    config: &Config,
    model: Option<&str>,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<ManagedChatModel> {
    // `model` may be a role alias (`hint:reasoning`) the registry keys the
    // route by; the provider only ever sees the concrete id it resolves to.
    managed_chat_model(
        role,
        config,
        model,
        None,
        native_tool_calling,
        thread_id,
        egress,
    )
}

/// Explicit-context counterpart for turn construction.
pub(crate) fn make_openhuman_backend_model_for_thread_in(
    extensions: Option<&crate::core::slots::Extensions>,
    role: &str,
    config: &Config,
    model: Option<&str>,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<ManagedChatModel> {
    managed_chat_model_for(
        extensions,
        role,
        config,
        model,
        None,
        native_tool_calling,
        thread_id,
        egress,
    )
}

/// The managed backend as a crate-native host `ChatModel`.
pub(crate) fn make_openhuman_backend_model(
    role: &str,
    config: &Config,
) -> anyhow::Result<ManagedChatModel> {
    managed_chat_model(role, config, None, None, true, None, &EgressLedger::none())
}

/// One-shot managed construction selected by an explicit dispatch context.
pub(crate) fn make_openhuman_backend_model_in(
    extensions: Option<&crate::core::slots::Extensions>,
    role: &str,
    config: &Config,
) -> anyhow::Result<ManagedChatModel> {
    managed_chat_model_for(
        extensions,
        role,
        config,
        None,
        None,
        true,
        None,
        &EgressLedger::none(),
    )
}
