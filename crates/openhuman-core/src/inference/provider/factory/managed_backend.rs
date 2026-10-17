//! The managed OpenHuman backend as a crate-native `ChatModel`: tier pinning per
//! workload role, `hint:*` translation, and the single managed egress emission.

use super::*;

/// Resolve the managed OpenHuman backend for `role` — the model id (tier /
/// summarization / default, with `hint:<tier>` translation) plus a configured
/// [`OpenHumanBackendModel`]. Shared by both the `Provider` path
/// ([`make_openhuman_backend`]) and the crate `ChatModel` path
/// ([`make_openhuman_backend_model`], issue #4727 Motion B).
pub(super) fn resolve_managed_backend(
    role: &str,
    config: &Config,
    egress: &EgressLedger,
) -> anyhow::Result<(OpenHumanBackendModel, String)> {
    resolve_managed_backend_with_model_override(role, config, None, egress)
}

/// Construct a managed turn model with the explicit OpenHuman thread attached.
/// This intentionally exposes no generic backend resolver outside the factory.
pub(crate) fn make_openhuman_backend_model_for_thread(
    role: &str,
    config: &Config,
    model: &str,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<(
    std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    String,
)> {
    // `model` may be a role alias (`hint:reasoning`) the registry keys the
    // route by; the backend only ever sees the concrete id it resolves to.
    let (backend, resolved_model) =
        resolve_managed_backend_with_model_override(role, config, Some(model), egress)?;
    Ok((
        std::sync::Arc::new(
            backend
                .with_native_tool_calling(native_tool_calling)
                .with_thread_id(thread_id),
        ),
        resolved_model,
    ))
}

pub(super) fn resolve_managed_backend_with_model_override(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
    egress: &EgressLedger,
) -> anyhow::Result<(OpenHumanBackendModel, String)> {
    // Managed construction shares the same privacy gate as local and BYOK
    // factories, before constructing a client or announcing any egress.
    enforce_local_only_inference(role, PROVIDER_OPENHUMAN)?;

    // Every managed role runs on one concrete model: the pinned default, else
    // `MODEL_MANAGED_DEFAULT`. Roles still matter — they picked the *route*
    // that led here — but there are no per-role tier endpoints any more.
    let model = managed_default_model(config);
    log::debug!(
        "[providers][chat-factory] role={} managed resolves to model={}",
        role,
        model
    );
    // Critical: pass the *config's* workspace directory through so the
    // provider's `AuthService` reads `auth-profiles.json` from the
    // same dir login wrote to. Without this, `ProviderRuntimeOptions::default()`
    // leaves `openhuman_dir = None`, the provider falls back to
    // `~/.openhuman`, and reads an unrelated (or empty)
    // profile store — surfacing as "No backend session: store a JWT
    // via auth (app-session)" even though login just succeeded in the
    // user's actual workspace (e.g. test workspaces under OPENHUMAN_WORKSPACE).
    let options = ProviderRuntimeOptions {
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        ..ProviderRuntimeOptions::default()
    };
    log::debug!(
        "[providers][chat-factory] building openhuman backend provider model={} state_dir={:?} secrets_encrypt={}",
        model,
        options.openhuman_dir,
        options.secrets_encrypt
    );
    // `model` is the managed default here, never a hint or a retired tier
    // slug; a caller's own `hint:*` / tier value only ever arrives through
    // `model_override`, translated below.
    // An override that is itself a managed alias (`hint:coding`, `hint:chat`)
    // means "this role on the managed backend" — the default model. A concrete
    // id (`openrouter/deepseek/deepseek-v4-pro`) is forwarded verbatim; the
    // backend is authoritative over its validity (issue #4598).
    let model = match model_override.map(str::trim).filter(|m| !m.is_empty()) {
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
    };

    // Egress spine (privacy epic S2, #4436): managed backend resolution is the
    // universal chokepoint for EVERY managed-backend inference construction —
    // the direct ChatModel path and both turn paths
    // (`create_turn_chat_model[_from_string]_with_native_tools`) resolve here.
    // Emitting once here guarantees the default managed chat turn discloses
    // egress exactly once (see `emit_inference_egress`).
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference("openhuman", &model, true),
    );
    Ok((
        OpenHumanBackendModel::new(config.api_url.as_deref(), &options, model.clone()),
        model,
    ))
}

/// The managed OpenHuman backend as a crate-native host `ChatModel`
/// ([`OpenHumanBackendModel`], issue #4727 Motion B) — the cutover replacement
/// for the `Provider` path. Same resolution; wraps the backend so the harness
/// holds a crate `ChatModel` and the dynamic JWT + `thread_id` + billing envelope
/// are bridged onto the crate wire client per call.
pub(crate) fn make_openhuman_backend_model(
    role: &str,
    config: &Config,
) -> anyhow::Result<(
    std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    String,
)> {
    let (model_client, model) = resolve_managed_backend(role, config, &EgressLedger::none())?;
    let chat: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        std::sync::Arc::new(model_client);
    Ok((chat, model))
}
