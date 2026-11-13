//! One-shot `create_chat_model*` entry points, the readiness probe, the
//! default-temperature wrapper, and the unresolved-route error builder.

use super::*;
#[cfg(not(test))]
use crate::inference::provider::factory::access_gates::verify_session_active;

/// Build an `Arc<dyn ChatModel>` for the given workload role.
///
/// The crate [`ChatModel`] is the model interface for the harness and one-shot
/// inference callers. Production and tests both inject this native interface;
/// `temperature` is applied as the request default while an explicit per-call
/// value still wins.
pub fn create_chat_model(
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    Ok(create_chat_model_with_model_id(role, config, temperature)?.0)
}

/// Like [`create_chat_model`], but also returns the resolved model id.
///
/// One-shot callers that persist or log the concrete model (e.g. the memory
/// summarise audit) need the id the role resolved to; the plain
/// [`create_chat_model`] drops it.
pub fn create_chat_model_with_model_id(
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let (model, model_id) = create_chat_model_with_model_id_inner(role, config)?;
    Ok((with_default_temperature(model, temperature), model_id))
}

pub(super) fn create_chat_model_with_model_id_inner(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    if let Some(model) = crate::core::runtime::CoreContext::current_host_overrides()
        .and_then(|local| local.model_for(role))
    {
        return Ok((
            model,
            config
                .default_model
                .clone()
                .unwrap_or_else(|| "host-model".into()),
        ));
    }
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
        if let Some(result) =
            try_create_claude_agent_sdk_chat_model(role, config, &EgressLedger::none())
        {
            return result;
        }
        if let Some(result) =
            try_create_claude_code_chat_model(role, config, None, &EgressLedger::none())
        {
            return result;
        }
        // Local OpenAI-compatible runtimes (Ollama / LM Studio / MLX / OMLX /
        // local-openai) → crate-native `ChatModel` (issue #4727 Motion B) instead
        // of a crate-adapted host provider. Cloud/BYOK/bespoke providers
        // return `None` here and fall through to the `Provider` path below.
        if let Some(result) =
            try_create_local_runtime_chat_model(role, config, &EgressLedger::none())
        {
            return result;
        }
        // Wire-equivalent BYOK cloud slugs (Anthropic / None / plain-Bearer, no
        // codex-oauth or `/v1/responses` fallback) → crate-native `ChatModel`
        // (issue #4727 Phase 3, conservative subset). `openai`/codex, custom
        // proxy slugs, and the managed entry return `None` and fall through.
        if let Some(result) = try_create_cloud_slug_chat_model(role, config, &EgressLedger::none())
        {
            return result;
        }
    }
    Err(unresolved_chat_model_error(
        role,
        &provider_for_role(role, config),
        config,
        &EgressLedger::none(),
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
        resolve_managed_backend(role, config, &EgressLedger::none()).map_err(|e| e.to_string())?;
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

pub(super) fn create_chat_model_from_string_with_model_id_inner(
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
        if let Some(result) = try_create_claude_agent_sdk_chat_model_from_string(
            role,
            &resolved,
            config,
            &EgressLedger::none(),
        ) {
            return result;
        }
        if let Some(result) = try_create_claude_code_chat_model_from_string(
            role,
            &resolved,
            config,
            None,
            &EgressLedger::none(),
        ) {
            return result;
        }
        if let Some(result) = try_create_local_runtime_chat_model_from_string(
            role,
            &resolved,
            config,
            &EgressLedger::none(),
        ) {
            return result;
        }
        if let Some(result) = try_create_cloud_slug_chat_model_from_string(
            role,
            &resolved,
            config,
            &EgressLedger::none(),
        ) {
            return result;
        }
    }
    Err(unresolved_chat_model_error(
        role,
        provider,
        config,
        &EgressLedger::none(),
    ))
}

pub(super) struct DefaultTemperatureChatModel {
    inner: Arc<dyn ChatModel<()>>,
    temperature: f64,
}

#[async_trait::async_trait]
impl ChatModel<()> for DefaultTemperatureChatModel {
    fn profile(&self) -> Option<&tinyinference_llm::model::ModelProfile> {
        self.inner.profile()
    }

    fn supports_input(
        &self,
        modality: tinyinference_llm::model::InputModality,
        mime: &str,
        source: tinyinference_llm::model::InputSource,
    ) -> bool {
        self.inner.supports_input(modality, mime, source)
    }

    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }

    async fn invoke(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        if request.temperature.is_none() {
            request.temperature = Some(self.temperature);
        }
        self.inner.invoke(state, request).await
    }

    async fn stream(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        if request.temperature.is_none() {
            request.temperature = Some(self.temperature);
        }
        self.inner.stream(state, request).await
    }
}

pub(super) fn with_default_temperature(
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
pub(super) fn unresolved_chat_model_error(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
) -> anyhow::Error {
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
            egress,
        );
    }

    #[cfg(not(test))]
    if let Err(error) = verify_session_active(config) {
        return error;
    }

    // Preserve the legacy chokepoint's disclosure ordering for invalid custom
    // routes: after both gates pass, the attempted external destination is
    // visible even when configuration validation then fails.
    emit_inference_egress(role, p, egress);

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
