
fn create_turn_chat_model_with_native_tools_and_route_inner(
    role: &str,
    config: &Config,
    model: &str,
    native_tool_calling: bool,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
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
            let (backend, _resolved_model) = resolve_managed_backend(role, config)?;
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
            emit_inference_egress(role, &format!("{CLAUDE_AGENT_SDK_PREFIX}{model}"));
            return Ok((
                Arc::new(ClaudeAgentSdkProvider::for_model(
                    config.claude_agent_sdk.clone(),
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
        ) {
            return result
                .map(|(chat, _configured_model)| (chat, provider_name.clone(), model.to_string()));
        }
        if let Some(result) = try_create_local_runtime_chat_model(role, config) {
            return result
                .map(|(chat, resolved_model)| (chat, provider_name.clone(), resolved_model));
        }
        if let Some(result) =
            try_create_cloud_slug_chat_model_with_native_tools(role, config, native_tool_calling)
        {
            return result
                .map(|(chat, resolved_model)| (chat, provider_name.clone(), resolved_model));
        }
    }
    Err(unresolved_chat_model_error(
        role,
        &provider_for_role(role, config),
        config,
    ))
}

/// Build the Claude Agent SDK subprocess directly as a crate model. This is a
/// prompt-guided model: TinyAgents owns its text-tool protocol, while the
/// provider owns only subprocess transport and NDJSON decoding.
fn try_create_claude_agent_sdk_chat_model(role: &str, config: &Config) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_claude_agent_sdk_chat_model_from_string(role, &resolved, config)
}

fn try_create_claude_agent_sdk_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
) -> OptionalChatModelResult {
    let model = match prepare_claude_agent_sdk_chat_model(role, provider, config)? {
        Ok(model) => model,
        Err(error) => return Some(Err(error)),
    };
    emit_inference_egress(role, &format!("{CLAUDE_AGENT_SDK_PREFIX}{model}"));
    let chat: Arc<dyn ChatModel<()>> = Arc::new(ClaudeAgentSdkProvider::for_model(
        config.claude_agent_sdk.clone(),
        model.clone(),
    ));
    Some(Ok((chat, model)))
}

fn prepare_claude_agent_sdk_chat_model(
    role: &str,
    provider: &str,
    config: &Config,
) -> Option<anyhow::Result<String>> {
    let model = claude_agent_sdk_model_from_string(provider, config)?;
    if let Err(error) = enforce_local_only_inference(role, provider) {
        return Some(Err(error));
    }
    #[cfg(not(test))]
    if let Err(error) = verify_session_active(config) {
        return Some(Err(error));
    }
    Some(Ok(model))
}

fn claude_agent_sdk_model_from_string(provider: &str, config: &Config) -> Option<String> {
    let provider = provider.trim();
    let model = if provider == CLAUDE_AGENT_SDK_PROVIDER {
        config.claude_agent_sdk.default_model.clone()
    } else if let Some(model) = provider.strip_prefix(CLAUDE_AGENT_SDK_PREFIX) {
        model.trim().to_string()
    } else {
        return None;
    };
    Some(model)
}

fn try_create_claude_code_chat_model(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_claude_code_chat_model_from_string(role, &resolved, config, model_override)
}

fn try_create_claude_code_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    model_override: Option<&str>,
) -> OptionalChatModelResult {
    let provider = provider.trim();
    let model_with_temp = provider
        .strip_prefix(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX)?;
    let (configured_model, temperature_override) = split_model_and_temperature(model_with_temp);
    if temperature_override.is_some() {
        log::warn!(
            "[providers][chat-factory] claude-code provider: per-model temperature override \
             is accepted but not wired through to the CLI — the @<temp> suffix is ignored"
        );
    }
    if configured_model.is_empty() {
        return Some(Err(anyhow::anyhow!(
            "[chat-factory] provider string '{}' for role '{}' has an empty model — \
             use 'claude-code:<model-id>'",
            provider,
            role
        )));
    }
    if let Err(error) = enforce_local_only_inference(role, provider) {
        return Some(Err(error));
    }
    #[cfg(not(test))]
    if let Err(error) = verify_session_active(config) {
        return Some(Err(error));
    }
    let workspace =
        crate::openhuman::inference::provider::claude_code::workspace_dir_from_config(config);
    let effective_model = model_override.unwrap_or(&configured_model).to_string();
    emit_inference_egress(
        role,
        &format!(
            "{}{effective_model}",
            crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX
        ),
    );
    let chat =
        match crate::openhuman::inference::provider::claude_code::ClaudeCodeProvider::from_env(
            effective_model,
            workspace,
            config.action_dir.clone(),
        ) {
            Ok(model) => Arc::new(model) as Arc<dyn ChatModel<()>>,
            Err(error) => return Some(Err(error)),
        };
    Some(Ok((chat, configured_model)))
}

/// Like [`create_turn_chat_model`] but for an **explicit** `provider_string` — the
/// explicit-string counterpart of [`create_turn_chat_model`], for producers
/// whose effective provider differs from the role's default resolution.
///
/// The triage path needs this: [`build_remote_provider`](crate::openhuman::agent::triage::routing)
/// forces the managed backend (`provider_string == `[`PROVIDER_OPENHUMAN`]) when the
/// subconscious route is local / BYOK-incomplete — the #1257 *"triage never goes
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
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
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
        let (backend, _resolved_model) = resolve_managed_backend(role, config)?;
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
    )
}

/// Local OpenAI-compatible runtimes (Ollama / LM Studio / MLX / OMLX /
/// local-openai) as a crate-native [`ChatModel`] (issue #4727).
///
/// Returns `None` when `role` does not resolve to a local runtime, allowing
/// [`create_chat_model_with_model_id`] to try cloud/BYOK/CLI constructors.
///
/// Endpoint/auth/`num_ctx` resolution uses the shared
/// `ollama_base_url_from_config` / `lm_studio_base_url` / profile helpers. It
/// runs the host access gates for custom/local providers —
/// [`enforce_local_only_inference`] (privacy mode) +
/// [`verify_session_active`] (session requirement) — so routing a local runtime
/// here cannot bypass either. Temperature rides the per-call `ModelRequest` on
/// the crate path (parity with the managed-backend cutover; the `@<temp>` suffix
/// still bakes a fixed override).
///
type ResolvedChatModel = (Arc<dyn ChatModel<()>>, String);
type OptionalChatModelResult = Option<anyhow::Result<ResolvedChatModel>>;

fn try_create_local_runtime_chat_model(role: &str, config: &Config) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_local_runtime_chat_model_from_string(role, &resolved, config, true)
}

fn try_create_local_runtime_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    require_session: bool,
) -> OptionalChatModelResult {
    use crate::openhuman::inference::local::profile::{
        LOCAL_OPENAI_PROFILE, MLX_PROFILE, OMLX_PROFILE,
    };

    let p = provider.trim().to_string();
    let is_local = p.starts_with(OLLAMA_PROVIDER_PREFIX)
        || p.starts_with(LM_STUDIO_PROVIDER_PREFIX)
        || p.starts_with(MLX_PROVIDER_PREFIX)
        || p.starts_with(OMLX_PROVIDER_PREFIX)
        || p.starts_with(LOCAL_OPENAI_PROVIDER_PREFIX);
    if !is_local {
        return None;
    }

    // Preserve host privacy-mode refusal + the session requirement for
    // custom/local providers.
    if let Err(e) = enforce_local_only_inference(role, &p) {
        return Some(Err(e));
    }
    if require_session {
        #[cfg(not(test))]
        if let Err(e) = verify_session_active(config) {
            return Some(Err(e));
        }
    }

    // Egress spine (privacy epic S2, #4436): committed to a local runtime here
    // (past the non-local `None` return + access gates). Disclose it as
    // NON-external — local inference never leaves the device, so
    // `emit_external_transfer` records it without firing a pending event. This
    // is the single local chokepoint for every ChatModel/turn entry.
    emit_inference_egress(role, &p);

    let unsupported = config.temperature_unsupported_models.clone();
    let empty_model_err = |p: &str, form: &str| {
        anyhow::anyhow!("[chat-factory] provider string '{p}' has an empty model — use '{form}'")
    };

    // Resolve the local `api_key` + auth style shared by lmstudio/omlx/local-openai
    // (Bearer when a key is configured, else no auth — same as the host builders).
    let keyed_auth = || {
        let api_key = config.local_ai.api_key.as_deref().unwrap_or("").to_string();
        let auth = if api_key.trim().is_empty() {
            CompatAuthStyle::None
        } else {
            CompatAuthStyle::Bearer
        };
        (api_key, auth)
    };
    // First env override, else `local_ai.base_url`, else the profile default.
    let env_or_config_url = |env: &str, default: &str| {
        std::env::var("OPENHUMAN_LOCAL_INFERENCE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var(env).ok().filter(|s| !s.trim().is_empty()))
            .or_else(|| config.local_ai.base_url.clone())
            .unwrap_or_else(|| default.to_string())
    };

    if let Some(rest) = p.strip_prefix(OLLAMA_PROVIDER_PREFIX) {
        let (model, temp) = split_model_and_temperature(rest);
        if model.is_empty() {
            return Some(Err(empty_model_err(&p, "ollama:<model-id>")));
        }
        // Ollama exposes the OpenAI-compatible endpoint at `/v1`.
        let base_url = crate::openhuman::inference::local::ollama_base_url_from_config(config);
        let normalized = base_url.trim_end_matches('/').trim_end_matches("/v1");
        let endpoint = format!("{normalized}/v1");
        let chat = super::crate_openai::make_crate_local_runtime_chat_model(
            "ollama",
            &endpoint,
            "",
            CompatAuthStyle::None,
            &model,
            &unsupported,
            temp,
            config.local_ai.num_ctx,
        );
        return Some(Ok((chat, model)));
    }
    if let Some(rest) = p.strip_prefix(LM_STUDIO_PROVIDER_PREFIX) {
        let (model, temp) = split_model_and_temperature(rest);
        if model.is_empty() {
            return Some(Err(empty_model_err(&p, "lmstudio:<model-id>")));
        }
        let endpoint = crate::openhuman::inference::local::lm_studio::lm_studio_base_url(config);
        let (api_key, auth) = keyed_auth();
        let chat = super::crate_openai::make_crate_local_runtime_chat_model(
            "lmstudio",
            &endpoint,
            &api_key,
            auth,
            &model,
            &unsupported,
            temp,
            None,
        );
        return Some(Ok((chat, model)));
    }
    if let Some(rest) = p.strip_prefix(MLX_PROVIDER_PREFIX) {
        let (model, temp) = split_model_and_temperature(rest);
        if model.is_empty() {
            return Some(Err(empty_model_err(&p, "mlx:<model-id>")));
        }
        let endpoint = env_or_config_url("MLX_SERVER_URL", MLX_PROFILE.default_base_url);
        let chat = super::crate_openai::make_crate_local_runtime_chat_model(
            "mlx",
            &endpoint,
            "",
            CompatAuthStyle::None,
            &model,
            &unsupported,
            temp,
            None,
        );
        return Some(Ok((chat, model)));
    }
    if let Some(rest) = p.strip_prefix(OMLX_PROVIDER_PREFIX) {
        let (model, temp) = split_model_and_temperature(rest);
        if model.is_empty() {
            return Some(Err(empty_model_err(&p, "omlx:<model-id>")));
        }
        let endpoint = env_or_config_url("OMLX_SERVER_URL", OMLX_PROFILE.default_base_url);
        let (api_key, auth) = keyed_auth();
        let chat = super::crate_openai::make_crate_local_runtime_chat_model(
            "omlx",
            &endpoint,
            &api_key,
            auth,
            &model,
            &unsupported,
            temp,
            None,
        );
        return Some(Ok((chat, model)));
    }
    if let Some(rest) = p.strip_prefix(LOCAL_OPENAI_PROVIDER_PREFIX) {
        let (model, temp) = split_model_and_temperature(rest);
        if model.is_empty() {
            return Some(Err(empty_model_err(&p, "local-openai:<model-id>")));
        }
        let endpoint = env_or_config_url("LOCAL_OPENAI_URL", LOCAL_OPENAI_PROFILE.default_base_url);
        let (api_key, auth) = keyed_auth();
        let chat = super::crate_openai::make_crate_local_runtime_chat_model(
            "local-openai",
            &endpoint,
            &api_key,
            auth,
            &model,
            &unsupported,
            temp,
            None,
        );
        return Some(Ok((chat, model)));
    }
    None
}

/// Build a crate-native local-runtime model for setup/probe calls that run
/// before the desktop session gate is established.
pub(crate) fn create_local_chat_model_from_string(
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    try_create_local_runtime_chat_model_from_string("chat", provider, config, false)
        .ok_or_else(|| anyhow::anyhow!("unsupported local provider string '{provider}'"))?
}

/// Verify the user has an active OpenHuman backend session.
///
/// Without this check, an unregistered user can configure every workload
/// to use a custom cloud provider and bypass the session requirement
/// entirely.  This function ensures that custom providers (Ollama,
/// `<slug>:<model>`) are only reachable when the workspace holds a valid
/// `app-session` JWT.
///
/// `pub(crate)`: also reused directly by the flows provider-connectivity
/// author gate (issue B45, `openhuman::flows::ops::evaluate_inference_readiness`)
/// as its Layer 1 sync session check, so the author-time gate and this
/// construction-time chokepoint can never diverge on what "session active"
/// means.
pub(crate) fn verify_session_active(config: &Config) -> anyhow::Result<()> {
    // An in-process library host is given its provider URL and credential by
    // its caller. It does not participate in the OpenHuman app's registration
    // or auth-profile lifecycle, so requiring a fabricated local session here
    // would mix desktop product policy into the library contract. The ambient
    // context is installed by the dispatch chokepoint; all other host kinds
    // retain the registration gate below.
    if !current_host_requires_session() {
        return Ok(());
    }

    verify_backend_session_active(config)
}

/// Managed `OpenhumanJwt` inference always needs the backend bearer, including
/// in a Library host. Library mode exempts caller-owned provider credentials;
/// it cannot manufacture a TinyHumans account credential.
pub(crate) fn verify_backend_session_active(config: &Config) -> anyhow::Result<()> {
    // Fast path: the scheduler gate already knows the session is dead.
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
        anyhow::bail!(
            "SESSION_EXPIRED: backend session not active — sign in to use custom providers"
        );
    }
    // Verify the app-session JWT actually exists in auth-profiles.
    let state_dir = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| std::path::PathBuf::from(".openhuman"))
        });
    let auth = AuthService::new(&state_dir, config.secrets.encrypt);
    let has_session = auth
        .get_provider_bearer_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            None,
        )?
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if !has_session {
        anyhow::bail!("SESSION_EXPIRED: no backend session — sign in to use OpenHuman")
    }
    Ok(())
}

/// Whether inference in the ambient host participates in OpenHuman app login.
/// Library hosts receive their provider configuration from the embedding
/// process; every other host retains the product session policy.
pub(crate) fn current_host_requires_session() -> bool {
    crate::core::runtime::context::CoreContext::current()
        .map(|ctx| host_requires_session(ctx.host_kind()))
        .unwrap_or(true)
}

fn host_requires_session(host_kind: crate::core::types::HostKind) -> bool {
    host_kind != crate::core::types::HostKind::Library
}

fn resolve_primary_cloud_provider_string(config: &Config) -> String {
    let primary = config
        .primary_cloud
        .as_deref()
        .and_then(|id| config.cloud_providers.iter().find(|entry| entry.id == id));

    if primary.is_some_and(is_openhuman_cloud_entry) {
        if let Some(legacy) = legacy_custom_inference_provider_string(config) {
            return legacy;
        }
        // Primary is explicitly OpenHuman but inference_url points at a custom
        // endpoint with no matching provider entry — this is a half-migrated BYOK
        // config. Fail closed so the user sees an actionable error rather than
        // silently routing through the managed backend.
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (host={}) \
                 but no matching cloud_providers entry found; returning fail-closed sentinel",
                redact_inference_url(config.inference_url.as_deref())
            );
            return BYOK_INCOMPLETE_SENTINEL.to_string();
        }
    }

    if let Some(entry) = primary {
        return cloud_entry_provider_string(entry, config);
    }

    // No explicit primary configured. If inference_url signals custom intent but
    // no matching provider entry exists, fail closed instead of falling back to
    // the managed backend.
    legacy_custom_inference_provider_string(config).unwrap_or_else(|| {
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (host={}) \
                 with no primary_cloud and no matching provider entry; returning fail-closed sentinel",
                redact_inference_url(config.inference_url.as_deref())
            );
            BYOK_INCOMPLETE_SENTINEL.to_string()
        } else {
            PROVIDER_OPENHUMAN.to_string()
        }
    })
}

/// Extract the host portion of an inference URL for safe logging.
///
/// Returns the host (e.g. `"api.example.com"`) so log lines are grep-friendly
/// without exposing tokens or credentials that may appear in query-string or
/// path components of a bearer-auth URL (e.g. `"https://host/v1?key=…"`).
/// Falls back to `"<redacted>"` when the URL cannot be parsed or is absent.
fn redact_inference_url(url: Option<&str>) -> &str {
    url.and_then(|u| {
        // Minimal host extraction: find the authority after "://".
        let after_scheme = u.find("://").map(|i| &u[i + 3..])?;
        // Authority ends at '/', '?', '#', or end-of-string.
        let host_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..host_end];
        // Strip optional "user:pass@" and port.
        let host = authority
            .rfind('@')
            .map_or(authority, |i| &authority[i + 1..]);
        let host = host.rfind(':').map_or(host, |i| &host[..i]);
        if host.is_empty() {
            None
        } else {
            Some(host)
        }
    })
    .unwrap_or("<redacted>")
}

/// Return `true` when the config contains a non-openhuman `inference_url`,
/// indicating the user intends custom/BYOK routing rather than the managed
/// backend.
fn has_custom_inference_intent(config: &Config) -> bool {
    config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .is_some_and(|url| !looks_like_openhuman_backend(url))
}

fn legacy_custom_inference_provider_string(config: &Config) -> Option<String> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| cloud_entry_provider_string(entry, config))
}
