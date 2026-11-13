//! Per-turn `create_turn_chat_model*` entry points pinned to an explicit model
//! (the `TurnModelSource` constructors), by role or by explicit provider string.

use super::*;

/// Build a crate-native [`ChatModel`] for the **turn path**, pinned to an explicit
/// `model` string — the turn's effective/dispatched model after any config-level
/// agent pin (issue #4249, Phase 3 P3-B). The per-`(role, model)` analogue of
/// [`create_chat_model_with_model_id`] used by the crate-native
/// [`TurnModelSource`](crate::agent::tinyagents::TurnModelSource) to construct
/// the primary + each workload-tier route directly.
///
/// - **Managed** → [`OpenHumanBackendModel`](super::openhuman_backend_model::OpenHumanBackendModel)
///   pinned to `model`; the backend resolves the tier from `request.model`, so a
///   tier alias / agent-model pin dispatches directly.
/// - **Local / cloud** → the crate builders; the model rides the role's resolved
///   provider string. A config-level *primary-model pin* on a local/cloud provider
///   is not re-pinned here (pins are tier selection on the managed backend); the
///   role's resolved model has the same behaviour.
/// - **Claude Agent SDK** → its direct prompt-guided [`ChatModel`] subprocess
///   adapter, pinned to `model`.
/// - **Claude Code** → its direct native-tool streaming [`ChatModel`] subprocess
///   adapter, pinned to `model`.
///
/// Respects the native test-model override, exactly as
/// [`create_chat_model_with_model_id`].
pub(crate) fn create_turn_chat_model(
    role: &str,
    config: &Config,
    model: &str,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_turn_chat_model_with_native_tools(role, config, model, temperature, true)
}

pub(crate) fn create_turn_chat_model_with_native_tools(
    role: &str,
    config: &Config,
    model: &str,
    temperature: f64,
    native_tool_calling: bool,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_turn_chat_model_with_native_tools_and_route(
        role,
        config,
        model,
        temperature,
        native_tool_calling,
        &EgressLedger::none(),
    )
    .map(|(chat, _, _)| chat)
}

/// Build a turn model together with the concrete provider and post-remap model
/// id that the constructed client will put on the wire. The route metadata is
/// consumed by channel audit recording; returning it from the construction
/// branches avoids re-parsing a provider string before cloud default-model and
/// abstract-tier remapping has run.
pub(crate) fn create_turn_chat_model_with_native_tools_and_route(
    role: &str,
    config: &Config,
    model: &str,
    temperature: f64,
    native_tool_calling: bool,
    egress: &EgressLedger,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
    create_turn_chat_model_with_native_tools_and_route_inner(
        role,
        config,
        model,
        native_tool_calling,
        egress,
    )
    .map(|(chat, provider, model)| (with_default_temperature(chat, temperature), provider, model))
}

pub(super) fn create_turn_chat_model_with_native_tools_and_route_inner(
    role: &str,
    config: &Config,
    model: &str,
    native_tool_calling: bool,
    egress: &EgressLedger,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
    if let Some(chat) = crate::core::runtime::CoreContext::current_host_overrides()
        .and_then(|local| local.model_for(role))
    {
        return Ok((chat, "host".into(), model.to_owned()));
    }
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if let Some(chat) = test_provider_override::current() {
        let provider = chat
            .profile()
            .and_then(|profile| profile.provider.clone())
            .unwrap_or_else(|| "injected".to_string());
        return Ok((chat, provider, model.to_string()));
    }
    let test_override_active = {
        #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
        {
            test_provider_override::current().is_some()
        }
        #[cfg(not(any(test, feature = "e2e-test-support", feature = "rss-bench")))]
        {
            false
        }
    };
    if !test_override_active {
        if resolves_to_managed_backend(role, config) {
            let (backend, _resolved_model) = resolve_managed_backend(role, config, egress)?;
            return Ok((
                Arc::new(
                    backend
                        .with_default_model(model)
                        .with_native_tool_calling(native_tool_calling),
                ),
                PROVIDER_OPENHUMAN.to_string(),
                model.to_string(),
            ));
        }
        let resolved_provider = provider_for_role(role, config);
        let provider_name = resolved_provider
            .trim()
            .split(':')
            .next()
            .unwrap_or(resolved_provider.trim())
            .to_string();
        if let Some(result) = prepare_claude_agent_sdk_chat_model(role, &resolved_provider, config)
        {
            let _resolved_model = result?;
            emit_inference_egress(role, &format!("{CLAUDE_AGENT_SDK_PREFIX}{model}"), egress);
            return Ok((
                Arc::new(ClaudeAgentSdkProvider::for_model(
                    (&config.claude_agent_sdk).into(),
                    model,
                )),
                provider_name,
                model.to_string(),
            ));
        }
        if let Some(result) = try_create_claude_code_chat_model_from_string(
            role,
            &resolved_provider,
            config,
            Some(model),
            egress,
        ) {
            return result
                .map(|(chat, _configured_model)| (chat, provider_name.clone(), model.to_string()));
        }
        if let Some(result) = try_create_local_runtime_chat_model(role, config, egress) {
            return result
                .map(|(chat, resolved_model)| (chat, provider_name.clone(), resolved_model));
        }
        if let Some(result) = try_create_cloud_slug_chat_model_with_native_tools(
            role,
            config,
            native_tool_calling,
            egress,
        ) {
            return result
                .map(|(chat, resolved_model)| (chat, provider_name.clone(), resolved_model));
        }
    }
    Err(unresolved_chat_model_error(
        role,
        &provider_for_role(role, config),
        config,
        egress,
    ))
}

/// Like [`create_turn_chat_model`] but for an **explicit** `provider_string` — the
/// explicit-string counterpart of [`create_turn_chat_model`], for producers
/// whose effective provider differs from the role's default resolution.
///
/// The triage path needs this: [`build_remote_provider`](crate::agent::triage::routing)
/// forces the managed backend (`provider_string == `[`PROVIDER_OPENHUMAN`]) when the
/// chat route is local / BYOK-incomplete — the #1257 *"triage never goes
/// local"* invariant — which a plain [`create_turn_chat_model`] (role → `provider_for_role`)
/// would violate by building the local model.
///
/// - `provider_string` empty / `"cloud"` / [`PROVIDER_OPENHUMAN`] → managed
///   [`OpenHumanBackendModel`] pinned to `model` (the force-managed case).
/// - Otherwise the string equals what the role resolves to (a BYOK cloud slug), so
///   this delegates to [`create_turn_chat_model`] for `role`.
///
/// Respects the test-provider override (bespoke/`Provider` path), like its siblings.
pub(crate) fn create_turn_chat_model_from_string(
    role: &str,
    provider_string: &str,
    config: &Config,
    model: &str,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_turn_chat_model_from_string_with_native_tools(
        role,
        provider_string,
        config,
        model,
        temperature,
        true,
    )
}

pub(crate) fn create_turn_chat_model_from_string_with_native_tools(
    role: &str,
    provider_string: &str,
    config: &Config,
    model: &str,
    temperature: f64,
    native_tool_calling: bool,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_turn_chat_model_from_string_with_native_tools_and_route(
        role,
        provider_string,
        config,
        model,
        temperature,
        native_tool_calling,
        &EgressLedger::none(),
    )
    .map(|(chat, _, _)| chat)
}

pub(crate) fn create_turn_chat_model_from_string_with_native_tools_and_route(
    role: &str,
    provider_string: &str,
    config: &Config,
    model: &str,
    temperature: f64,
    native_tool_calling: bool,
    egress: &EgressLedger,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
    if let Some(chat) = crate::core::runtime::CoreContext::current_host_overrides()
        .and_then(|local| local.model_for(role))
    {
        return Ok((chat, "host".into(), model.to_owned()));
    }
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if let Some(chat) = test_provider_override::current() {
        let provider = chat
            .profile()
            .and_then(|profile| profile.provider.clone())
            .unwrap_or_else(|| "injected".to_string());
        return Ok((
            with_default_temperature(chat, temperature),
            provider,
            model.to_string(),
        ));
    }
    let test_override_active = {
        #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
        {
            test_provider_override::current().is_some()
        }
        #[cfg(not(any(test, feature = "e2e-test-support", feature = "rss-bench")))]
        {
            false
        }
    };
    let p = provider_string.trim();
    let is_managed = p.is_empty() || p == "cloud" || p == PROVIDER_OPENHUMAN;
    if is_managed && !test_override_active {
        let (backend, _resolved_model) = resolve_managed_backend(role, config, egress)?;
        return Ok((
            with_default_temperature(
                Arc::new(
                    backend
                        .with_default_model(model)
                        .with_native_tool_calling(native_tool_calling),
                ),
                temperature,
            ),
            PROVIDER_OPENHUMAN.to_string(),
            model.to_string(),
        ));
    }
    // A concrete non-managed string equals the role's resolution (triage only
    // honours a BYOK **cloud** route as-is), so the role-based builder matches.
    create_turn_chat_model_with_native_tools_and_route(
        role,
        config,
        model,
        temperature,
        native_tool_calling,
        egress,
    )
}
