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

/// The concrete model id a managed route runs on: the pinned default (else
/// `MODEL_MANAGED_DEFAULT`), unless `model_override` names something.
///
/// An override that is itself a managed alias (`hint:coding`, `hint:chat`)
/// means "this role on the managed backend": the default model. A concrete id
/// (`openrouter/deepseek/deepseek-v4-pro`) is forwarded verbatim; the backend
/// is authoritative over its validity (issue #4598).
pub(super) fn resolve_managed_model(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
) -> String {
    // Every managed role runs on one concrete model: the pinned default, else
    // `MODEL_MANAGED_DEFAULT`. Roles still matter (they picked the *route* that
    // led here) but there are no per-role tier endpoints any more.
    let model = managed_default_model(config);
    log::debug!(
        "[providers][chat-factory] role={} managed resolves to model={}",
        role,
        model
    );
    match model_override.map(str::trim).filter(|m| !m.is_empty()) {
        Some(raw) if is_known_openhuman_tier(raw) => {
            log::debug!(
                "[providers][chat-factory] role={} override '{}' is a managed alias; using model={}",
                role,
                raw,
                model
            );
            model
        }
        Some(raw) => {
            log::debug!(
                "[providers][chat-factory] role={} forwarding pinned model '{}' verbatim to the managed backend",
                role,
                raw
            );
            raw.to_string()
        }
        None => model,
    }
}

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
    enforce_local_only_inference(role, PROVIDER_OPENHUMAN)?;
    let Some(entry) = crate::inference::registry::chat_model(PROVIDER_OPENHUMAN) else {
        log::debug!("[providers][chat-factory] role={role} managed provider is not registered");
        return Err(managed_unavailable_error());
    };
    let model = resolve_managed_model(role, config, model_override);
    log::debug!(
        "[providers][chat-factory] role={role} managed provider served by a registered chat model model={model}"
    );
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference("openhuman", &model, true),
    );
    (entry.build)(&crate::inference::registry::ChatModelRequest {
        role,
        config,
        model_override: Some(&model),
        default_model,
        native_tool_calling,
        thread_id,
        egress,
    })
}

/// Construct a managed turn model with the explicit OpenHuman thread attached.
pub(crate) fn make_openhuman_backend_model_for_thread(
    role: &str,
    config: &Config,
    model: &str,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<ManagedChatModel> {
    // `model` may be a role alias (`hint:reasoning`) the registry keys the
    // route by; the provider only ever sees the concrete id it resolves to.
    managed_chat_model(
        role,
        config,
        Some(model),
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

/// The managed model for a caller that holds runtime options rather than a
/// [`Config`] (the configured `delegate` tool): `model` is used verbatim, with
/// no tier translation and no egress emission, exactly as that tool always did.
pub(crate) fn make_openhuman_backend_model_for_options(
    options: &crate::inference::provider::ProviderRuntimeOptions,
    model: &str,
    thread_id: Option<&str>,
) -> anyhow::Result<ManagedChatModel> {
    let Some(entry) = crate::inference::registry::chat_model(PROVIDER_OPENHUMAN) else {
        return Err(managed_unavailable_error());
    };
    let mut config = Config::default();
    if let Some(dir) = options.openhuman_dir.as_ref() {
        config.config_path = dir.join("config.toml");
    }
    config.secrets.encrypt = options.secrets_encrypt;
    (entry.build)(&crate::inference::registry::ChatModelRequest {
        role: "delegate",
        config: &config,
        model_override: Some(model),
        default_model: Some(model),
        native_tool_calling: true,
        thread_id,
        egress: &EgressLedger::none(),
    })
}
