
fn create_chat_model_with_model_id_inner(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if let Some(model) = test_provider_override::current() {
        return Ok((model, "mock-model".to_string()));
    }
    // Managed OpenHuman backend → crate-native host `ChatModel`
    // ([`OpenHumanBackendModel`], issue #4727 Motion B) instead of a
    // adapted provider. A native test-model override must still win, so only
    // take this path when no
    // override is installed. The public wrapper supplies the construction-time
    // default while preserving an explicit per-call `ModelRequest` temperature.
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
            return make_openhuman_backend_model(role, config);
        }
        if let Some(result) = try_create_claude_agent_sdk_chat_model(role, config) {
            return result;
        }
        if let Some(result) = try_create_claude_code_chat_model(role, config, None) {
            return result;
        }
        // Local OpenAI-compatible runtimes (Ollama / LM Studio / MLX / OMLX /
        // local-openai) → crate-native `ChatModel` (issue #4727 Motion B) instead
        // of a crate-adapted host provider. Cloud/BYOK/bespoke providers
        // return `None` here and fall through to the `Provider` path below.
        if let Some(result) = try_create_local_runtime_chat_model(role, config) {
            return result;
        }
        // Wire-equivalent BYOK cloud slugs (Anthropic / None / plain-Bearer, no
        // codex-oauth or `/v1/responses` fallback) → crate-native `ChatModel`
        // (issue #4727 Phase 3, conservative subset). `openai`/codex, custom
        // proxy slugs, and the managed entry return `None` and fall through.
        if let Some(result) = try_create_cloud_slug_chat_model(role, config) {
            return result;
        }
    }
    Err(unresolved_chat_model_error(
        role,
        &provider_for_role(role, config),
        config,
    ))
}

/// Whether `role` resolves to the managed OpenHuman backend (vs BYOK / local /
/// claude-code). Uses the same empty/`cloud`/`openhuman` normalization as
/// [`create_chat_model_from_string`] so every managed role shares one path.
pub(crate) fn resolves_to_managed_backend(role: &str, config: &Config) -> bool {
    let mut resolved = provider_for_role(role, config);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed == "cloud" {
        resolved = resolve_primary_cloud_provider_string(config);
    }
    resolved.trim() == PROVIDER_OPENHUMAN
}

/// Probe whether `role` can actually complete an inference call right now
/// (issue B45 — the flows provider-connectivity author gate).
///
/// Two-stage check, mirroring the two ways a `role` can be un-runnable:
///
/// 1. **Construction** — [`create_chat_model_with_model_id_inner`] must
///    succeed. This is the existing Layer 1 check (BYOK-incomplete config,
///    unknown provider slug, local-only privacy-mode block, …) reused
///    verbatim so this probe never re-implements it.
/// 2. **Managed-backend readiness** — when `role` resolves to the managed
///    OpenHuman backend, [`OpenHumanBackendModel::probe_readiness`] makes one
///    cheap real completion attempt to catch the "account has no provider API
///    key configured" class of failure that construction alone cannot see
///    (construction only builds the client; it never calls the backend).
///    BYOK/local models have no such hidden failure mode — their construction
///    step already validates what it can, so they return `Ok(())` here
///    unconditionally.
///
/// Respects the [`test_provider_override`] test seam: when a mock model is
/// installed, construction returns it immediately and this function returns
/// `Ok(())` without ever touching the network or resolving `role` again —
/// `resolves_to_managed_backend` is a pure config read that would otherwise
/// still call this "managed" in a test with a bare default `Config`.
pub async fn probe_inference_readiness(role: &str, config: &Config) -> Result<(), String> {
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if test_provider_override::current().is_some() {
        log::debug!(
            "[flows][inference-probe] role={role} test model override active — skipping probe"
        );
        return Ok(());
    }

    log::debug!("[flows][inference-probe] role={role} verifying model construction");
    if let Err(e) = create_chat_model_with_model_id_inner(role, config) {
        log::debug!("[flows][inference-probe] role={role} construction failed: {e}");
        return Err(e.to_string());
    }

    if !resolves_to_managed_backend(role, config) {
        log::debug!(
            "[flows][inference-probe] role={role} resolves to a non-managed provider — \
             construction succeeded, nothing further to probe"
        );
        return Ok(());
    }

    log::debug!(
        "[flows][inference-probe] role={role} resolves to the managed OpenHuman backend — \
         probing readiness"
    );
    let (managed_model, model_id) =
        resolve_managed_backend(role, config).map_err(|e| e.to_string())?;
    let result = managed_model.probe_readiness().await;
    log::debug!(
        "[flows][inference-probe] role={role} model={model_id} probe result: {}",
        if result.is_ok() { "ready" } else { "not ready" }
    );
    result
}

/// Build an `Arc<dyn ChatModel>` from an explicit provider string and config.
///
/// The explicit-string counterpart of [`create_chat_model`].
pub fn create_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_chat_model_from_string_with_model_id(role, provider, config, temperature)
        .map(|(model, _)| model)
}

/// Build a crate [`ChatModel`] from an explicit provider string and return the
/// concrete model id selected by that provider.
///
/// Managed, local-runtime, configured cloud-slug, Claude SDK/Code, and Codex
/// strings all construct native `ChatModel` implementations directly.
pub fn create_chat_model_from_string_with_model_id(
    role: &str,
    provider: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let (model, model_id) =
        create_chat_model_from_string_with_model_id_inner(role, provider, config)?;
    Ok((with_default_temperature(model, temperature), model_id))
}

fn create_chat_model_from_string_with_model_id_inner(
    role: &str,
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if let Some(model) = test_provider_override::current() {
        return Ok((model, "mock-model".to_string()));
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
        let mut resolved = provider.trim().to_string();
        if resolved.is_empty() || resolved == "cloud" {
            resolved = resolve_primary_cloud_provider_string(config);
        }
        if resolved == PROVIDER_OPENHUMAN {
            return make_openhuman_backend_model(role, config);
        }
        if let Some(result) =
            try_create_claude_agent_sdk_chat_model_from_string(role, &resolved, config)
        {
            return result;
        }
        if let Some(result) =
            try_create_claude_code_chat_model_from_string(role, &resolved, config, None)
        {
            return result;
        }
        if let Some(result) =
            try_create_local_runtime_chat_model_from_string(role, &resolved, config, true)
        {
            return result;
        }
        if let Some(result) = try_create_cloud_slug_chat_model_from_string(role, &resolved, config)
        {
            return result;
        }
    }
    Err(unresolved_chat_model_error(role, provider, config))
}

struct DefaultTemperatureChatModel {
    inner: Arc<dyn ChatModel<()>>,
    temperature: f64,
}

#[async_trait::async_trait]
impl ChatModel<()> for DefaultTemperatureChatModel {
    fn profile(&self) -> Option<&tinyinference::model::ModelProfile> {
        self.inner.profile()
    }

    async fn invoke(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        if request.temperature.is_none() {
            request.temperature = Some(self.temperature);
        }
        self.inner.invoke(state, request).await
    }

    async fn stream(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        if request.temperature.is_none() {
            request.temperature = Some(self.temperature);
        }
        self.inner.stream(state, request).await
    }
}

fn with_default_temperature(
    model: Arc<dyn ChatModel<()>>,
    temperature: f64,
) -> Arc<dyn ChatModel<()>> {
    Arc::new(DefaultTemperatureChatModel {
        inner: model,
        temperature,
    })
}

/// Reproduce the legacy provider factory's access gates and diagnostics for a
/// provider string that none of the crate-native model constructors accepted.
///
/// Successful production routes never reach this function. Keeping error
/// resolution separate means `create_chat_model*` no longer constructs a
/// legacy `Provider` merely to discover that a route is invalid.
fn unresolved_chat_model_error(role: &str, provider: &str, config: &Config) -> anyhow::Error {
    let p = provider.trim();

    if let Err(error) = enforce_local_only_inference(role, p) {
        return error;
    }

    if p == BYOK_INCOMPLETE_SENTINEL {
        let inference_url = config
            .inference_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("<unset>");
        return anyhow::anyhow!(
            "[chat-factory] BYOK_INCOMPLETE: inference_url is set to a custom/direct endpoint \
             ({inference_url}) but no matching cloud_providers entry was found for role '{role}'. \
             To complete BYOK setup add a cloud_providers entry whose endpoint matches \
             {inference_url} (or use a workload-specific route). \
             To use the OpenHuman managed backend instead, clear inference_url from config."
        );
    }

    if p.is_empty() || p == "cloud" {
        return unresolved_chat_model_error(
            role,
            &resolve_primary_cloud_provider_string(config),
            config,
        );
    }

    #[cfg(not(test))]
    if let Err(error) = verify_session_active(config) {
        return error;
    }

    // Preserve the legacy chokepoint's disclosure ordering for invalid custom
    // routes: after both gates pass, the attempted external destination is
    // visible even when configuration validation then fails.
    emit_inference_egress(role, p);

    if let Some((slug, model_with_temperature)) = p.split_once(':') {
        if slug.trim().is_empty() {
            return anyhow::anyhow!(
                "[chat-factory] provider string '{}' for role '{}' has an empty slug",
                p,
                role
            );
        }
        let (model, _) = split_model_and_temperature(model_with_temperature);
        return match resolve_cloud_slug(role, slug.trim(), &model, config) {
            Err(error) => error,
            Ok(_) => anyhow::anyhow!(
                "[chat-factory] configured provider '{}' for role '{}' did not produce a crate-native chat model",
                p,
                role
            ),
        };
    }

    anyhow::anyhow!(
        "[chat-factory] unrecognised provider string '{}' for role '{}'. \
         Valid forms: openhuman, ollama:<model>, lmstudio:<model>, mlx:<model>, omlx:<model>, \
         local-openai:<model>, claude_agent_sdk, claude_agent_sdk:<model>, <slug>:<model>. \
         Configured slugs: [{}]",
        p,
        role,
        config
            .cloud_providers
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Canonical managed-backend tier for a specialised workload role.
///
/// The managed backend otherwise derives its model from `config.default_model`
/// (which defaults to the `chat-v1` tier), so a tier-specific workload whose
/// per-workload provider is unset would silently inherit the global default —
/// e.g. the `code_executor` sub-agent (`hint = "coding"`) would run on `chat-v1`
/// instead of the dedicated `coding-v1` tier, defeating the whole point of the
/// hint. The `hint:<tier>` translation in [`make_openhuman_backend`] only fires
/// when the *model string itself* is `hint:coding`; here the model originates
/// from `default_model`, so the workload role is the only signal left and must
/// be mapped explicitly.
///
/// Returns `Some(tier)` for the specialised roles that map 1:1 to a managed
/// tier (`reasoning`, `agentic`, `coding`, `vision`, `subconscious`). Returns
/// `None` for:
///
/// - the generic `chat` role (and any other background/unknown role), which
///   keeps inheriting `default_model`: the front-line chat turn and legacy
///   `default_model = "reasoning-v1"` installs deliberately fall through to the
///   `chat` role (see the session builder) and rely on `default_model` driving
///   the model — pinning `chat` here would regress them.
/// - `summarization` / `memory`, which are pinned in a dedicated branch of
///   [`make_openhuman_backend`] via [`summarization_tier_model`] (fixed at
///   `summarization-v1`) rather than here, only so the `memory` alias and the
///   role string share one resolution site. They do **not** fall through to
///   `default_model`.
///
/// `subconscious` IS pinned (to the lightweight `chat-v1` tier) even though it
/// is a background workload: the cloud subconscious tick builds via the session
/// builder with `default_model = "hint:subconscious"` (a role-routing marker, not
/// a real tier), so "inherit `default_model`" would forward that marker to the
/// backend. Pinning here resolves the managed model declaratively to `chat-v1` —
/// the cheap monitoring tier the workload wants — independent of `default_model`,
/// while [`provider_for_role`] still lets `subconscious_provider` choose the
/// provider (managed / BYOK / local).
///
/// For `vision` the default-inheritance mismatch is not just suboptimal but
/// fatal: an unset `vision_provider` would resolve to `chat-v1`,
/// `model_supports_vision` would report `false`, and the turn engine would strip
/// every attached image — leaving the managed vision sub-agent blind.
fn managed_tier_for_role(role: &str) -> Option<&'static str> {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_V1,
        MODEL_VISION_V1,
    };
    match role {
        "reasoning" => Some(MODEL_REASONING_V1),
        "agentic" => Some(MODEL_AGENTIC_V1),
        "coding" => Some(MODEL_CODING_V1),
        // Burst rides the managed backend's high-throughput tier. Pinned here
        // (rather than collapsing to `default_model`) so `hint = "burst"`
        // workers actually reach `burst-v1`.
        // There is no `burst_provider` knob: burst is managed-only.
        "burst" => Some(MODEL_BURST_V1),
        "vision" => Some(MODEL_VISION_V1),
        // Background subconscious tick/triage: pinned to the lightweight chat
        // tier (see the doc above for why it is pinned despite being background).
        "subconscious" => Some(MODEL_CHAT_V1),
        _ => None,
    }
}

/// The **managed-backend** summarization tier model — fixed at
/// [`MODEL_SUMMARIZATION_V1`] (`summarization-v1`).
///
/// Read **only** on the managed OpenHuman path (inside [`make_openhuman_backend`]),
/// so it is consumed iff the `summarization`/`memory` role actually resolves to
/// the managed backend — BYOK and local routes carry their own model in the
/// provider string and never reach here.
///
/// The managed summarization tier is intentionally **not** user-overridable: the
/// hosted backend serves exactly one tier (`summarization-v1`) for this workload,
/// so there is nothing else valid to point it at. Users who want a different
/// model run summarization on a BYOK/local `memory_provider`, where the model
/// rides in the provider string. (`memory_tree.cloud_llm_model` is no longer
/// consumed — see its config doc.)
pub(crate) fn summarization_tier_model() -> &'static str {
    crate::openhuman::config::MODEL_SUMMARIZATION_V1
}

/// Build the OpenHuman backend provider (session-JWT auth).
///
/// `role` is the workload name (e.g. `"chat"`, `"coding"`, `"vision"`). A
/// specialised workload role is pinned to its canonical managed tier via
/// [`managed_tier_for_role`] so the `hint = "..."` a sub-agent declares actually
/// reaches the matching backend tier instead of collapsing to `default_model`.
/// The `summarization`/`memory` roles resolve their tier from
/// [`summarization_tier_model`] (fixed at `summarization-v1`) so they never
/// collapse to `default_model`. The generic `chat` role (and background roles)
/// keep inheriting `config.default_model`.
/// Resolve the managed OpenHuman backend for `role` — the model id (tier /
/// summarization / default, with `hint:<tier>` translation) plus a configured
/// [`OpenHumanBackendModel`]. Shared by both the `Provider` path
/// ([`make_openhuman_backend`]) and the crate `ChatModel` path
/// ([`make_openhuman_backend_model`], issue #4727 Motion B).
fn resolve_managed_backend(
    role: &str,
    config: &Config,
) -> anyhow::Result<(OpenHumanBackendModel, String)> {
    resolve_managed_backend_with_model_override(role, config, None)
}

fn resolve_managed_backend_with_model_override(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
) -> anyhow::Result<(OpenHumanBackendModel, String)> {
    let model = if let Some(tier) = managed_tier_for_role(role) {
        log::debug!(
            "[providers][chat-factory] role={} pinned to managed tier model={}",
            role,
            tier
        );
        tier.to_string()
    } else if matches!(role, "summarization" | "memory") {
        // Managed summarization/memory tier — fixed at `summarization-v1` rather
        // than inherited from `config.default_model`, so every managed
        // summarization caller — the memory tree, the chat-turn payload
        // summarizer, meeting summaries, and any `hint = "summarization"`
        // sub-agent — reaches the dedicated tier instead of silently collapsing
        // to `chat-v1`. BYOK/local routes never reach here — they build from the
        // provider string.
        let tier = summarization_tier_model().to_string();
        log::debug!(
            "[providers][chat-factory] role={} resolved managed summarization tier model={}",
            role,
            tier
        );
        tier
    } else {
        config
            .default_model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| "reasoning-v1".to_string())
    };
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
    // Translate `hint:<tier>` model strings into the OpenHuman backend's
    // canonical tier names.  Unrecognised `hint:*` strings (e.g. `hint:reaction`
    // for lightweight models) are forwarded as-is — the backend is authoritative
    // over which hint values it accepts, and the web-chat model_override path
    // uses these verbatim.  Only non-hint strings that are not a known canonical
    // tier (stale `default_model` values written by older UI versions, e.g.
    // "deepseek-v4-pro", "claude-opus-4-7") fall back to the platform default.
    let model = match model.strip_prefix("hint:") {
        Some("reasoning") => crate::openhuman::config::MODEL_REASONING_V1.to_string(),
        Some("chat") => crate::openhuman::config::MODEL_CHAT_V1.to_string(),
        Some("agentic") => crate::openhuman::config::MODEL_AGENTIC_V1.to_string(),
        Some("burst") => crate::openhuman::config::MODEL_BURST_V1.to_string(),
        Some("coding") => crate::openhuman::config::MODEL_CODING_V1.to_string(),
        Some("summarization") => crate::openhuman::config::MODEL_SUMMARIZATION_V1.to_string(),
        Some("vision") => crate::openhuman::config::MODEL_VISION_V1.to_string(),
        Some(_) => {
            // Unrecognised hint — forward verbatim; the backend decides validity.
            model
        }
        None => {
            // `model` is guaranteed non-empty here: an empty/whitespace
            // `default_model` was already normalised to `reasoning-v1` above, and
            // the managed-tier / summarization branches yield non-empty tier
            // constants. So a non-`hint:` id is either a known canonical tier or a
            // raw/BYOK id the user pinned — both forward verbatim; only the log
            // line differs.
            if is_known_openhuman_tier(&model) {
                model
            } else {
                // Unrecognised NON-empty model id — a raw/BYOK model the user
                // pinned (e.g. `claude-opus-4`, written into `default_model` or
                // a per-agent model pin). Forward it verbatim so the selected
                // model actually reaches provider construction instead of the
                // core silently collapsing it onto `reasoning-v1`. The managed
                // backend is authoritative over validity and returns a clear
                // error for a genuinely bad id (issue #4598).
                log::debug!(
                    "[providers][chat-factory] forwarding raw/BYOK model '{}' verbatim to the \
                     OpenHuman backend (not a managed tier); the backend validates it",
                    model
                );
                model
            }
        }
    };
    let model = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(model);

    // Egress spine (privacy epic S2, #4436): managed backend resolution is the
    // universal chokepoint for EVERY managed-backend inference construction —
    // the direct ChatModel path and both turn paths
    // (`create_turn_chat_model[_from_string]_with_native_tools`) resolve here.
    // Emitting once here guarantees the default managed chat turn discloses
    // egress exactly once (see `emit_inference_egress`).
    crate::openhuman::security::egress::emit_external_transfer(
        crate::openhuman::security::egress::EgressDescriptor::inference("openhuman", &model, true),
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
    std::sync::Arc<dyn tinyinference::model::ChatModel<()>>,
    String,
)> {
    let (model_client, model) = resolve_managed_backend(role, config)?;
    let chat: std::sync::Arc<dyn tinyinference::model::ChatModel<()>> =
        std::sync::Arc::new(model_client);
    Ok((chat, model))
}

/// Build a crate-native [`ChatModel`] for the **turn path**, pinned to an explicit
/// `model` string — the turn's effective/dispatched model after any config-level
/// agent pin (issue #4249, Phase 3 P3-B). The per-`(role, model)` analogue of
/// [`create_chat_model_with_model_id`] used by the crate-native
/// [`TurnModelSource`](crate::openhuman::agent::tinyagents::TurnModelSource) to construct
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
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String, String)> {
    create_turn_chat_model_with_native_tools_and_route_inner(
        role,
        config,
        model,
        native_tool_calling,
    )
    .map(|(chat, provider, model)| (with_default_temperature(chat, temperature), provider, model))
}
