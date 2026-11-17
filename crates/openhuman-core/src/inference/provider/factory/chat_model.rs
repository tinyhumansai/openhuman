//! One-shot `create_chat_model*` entry points, the readiness probe, the
//! default-temperature wrapper, and the unresolved-route error builder.

#[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
use super::test_provider_override;
use super::{
    access_gates::{emit_inference_egress, enforce_local_only_inference},
    cloud_slug::resolve_cloud_slug_with_access,
    credentials::CredentialAccess,
    local_runtime::try_create_local_runtime_chat_model_from_string,
    provider_for_role,
    routing::route_with_model,
    subprocess_providers::OpenHumanMcpEndpoint,
    BYOK_INCOMPLETE_SENTINEL, CLAUDE_AGENT_SDK_PREFIX, CLAUDE_AGENT_SDK_PROVIDER,
    PROVIDER_OPENHUMAN,
};
use crate::config::ops::resolve_primary_cloud_provider_string;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::security::egress::EgressLedger;
use std::sync::Arc;
use tinyagents_harness::host::{
    BuildOpts, ModelBuildGate, ModelBuildRequest, ModelFactory, ModelPurpose,
};
use tinyinference_llm::model::{
    ChatModel, ModelRequestDefaults, ProfileOverrideModel, ResolvedModelRoute,
};
use tinyinference_providers::route::split_model_and_temperature;
use tinyinference_providers::route::{ProviderRoute, RouteKind};

/// OpenHuman's host policy and construction adapter for a harness turn.
///
/// The harness owns turn assembly; this adapter resolves OpenHuman provider
/// configuration and enforces product access policy for each requested model.
pub(crate) struct OpenHumanModelFactory<'a> {
    pub(crate) ctx: &'a DispatchCtx,
    pub(crate) config: &'a Config,
    pub(crate) primary_provider: Option<&'a str>,
    pub(crate) egress: &'a EgressLedger,
    pub(crate) thread_id: Option<&'a str>,
    /// Attach OpenHuman's per-turn multimodal providers using the route this
    /// factory actually selected. One-shot probes do not need host attachments.
    pub(crate) attach_host_attachments: bool,
    pub(crate) credentials: super::credentials::RuntimeCredentialSnapshot,
    /// Spend controls placed beneath attachment rendering on every model this
    /// factory builds, filled once the turn is admitted.
    pub(crate) host_controls: Option<&'a crate::inference::host_controls::HostControlSlot>,
    /// `[context] summarizer_model`: the model a summarization request calls
    /// on its own provider route instead of the route's model.
    pub(crate) summarizer_model: Option<&'a str>,
}

impl OpenHumanModelFactory<'_> {
    fn route_name<'a>(request: &'a ModelBuildRequest, role: &'a str) -> &'a str {
        request
            .route
            .as_deref()
            .or_else(|| request.model_pin.as_deref().filter(|pin| !pin.is_empty()))
            .unwrap_or(role)
    }

    pub(super) fn provider_for(&self, request: &ModelBuildRequest) -> String {
        match &request.purpose {
            ModelPurpose::Primary | ModelPurpose::Summarization => {
                let provider = self
                    .primary_provider
                    .map(str::to_owned)
                    .unwrap_or_else(|| provider_for_role(&request.role, self.config));
                let summarizes = matches!(request.purpose, ModelPurpose::Summarization);
                route_with_model(provider, self.summarizer_model.filter(|_| summarizes))
            }
            ModelPurpose::WorkloadTier(_) => provider_for_role(&request.role, self.config),
        }
    }

    pub(super) fn injected(&self, request: &ModelBuildRequest) -> Option<Arc<dyn ChatModel<()>>> {
        let route = request.route.as_deref();
        let host_model = self
            .ctx
            .host_overrides()
            .and_then(|local| match &request.purpose {
                ModelPurpose::Primary => local.model_for(&request.role),
                ModelPurpose::Summarization => local
                    .role_models
                    .get("summarization")
                    .cloned()
                    .or_else(|| local.model_for(&request.role)),
                ModelPurpose::WorkloadTier(_) => route
                    .and_then(|route| local.role_models.get(route))
                    .or_else(|| local.role_models.get(&request.role))
                    .cloned(),
            });
        if host_model.is_some() {
            return host_model;
        }
        #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
        if let Some(model) = test_provider_override::current() {
            return Some(model);
        }
        None
    }

    async fn build_request(
        &self,
        request: &ModelBuildRequest,
    ) -> anyhow::Result<(Arc<dyn ChatModel<()>>, ResolvedModelRoute)> {
        let role = request.role.as_str();
        let pin = request.model_pin.as_deref().unwrap_or("");
        let provider = self.provider_for(request);
        if let Some(model) = self.injected(request) {
            let profile = model.profile().cloned().unwrap_or_default();
            let route = ResolvedModelRoute::new(
                profile.provider.as_deref().unwrap_or("injected"),
                profile.model.as_deref().unwrap_or(pin),
                Self::route_name(request, role),
            );
            return Ok((model, route));
        }

        let provider_route = ProviderRoute::parse(&provider);
        let managed = match (&request.purpose, self.primary_provider) {
            (ModelPurpose::Primary | ModelPurpose::Summarization, Some(provider)) => {
                let provider = provider.trim();
                provider.is_empty()
                    || provider == "cloud"
                    || provider == PROVIDER_OPENHUMAN
                    || provider_route
                        .as_ref()
                        .is_some_and(|route| matches!(route.kind(), RouteKind::Managed))
            }
            _ => resolves_to_managed_backend(role, self.config),
        };
        if managed {
            let resolved = self.ctx.resolved();
            let extensions = resolved
                .as_deref()
                .map(crate::core::runtime::CoreContext::extensions);
            let (model, concrete) =
                super::managed_backend::make_openhuman_backend_model_for_thread_in(
                    extensions,
                    role,
                    self.config,
                    (!pin.is_empty()).then_some(pin),
                    request.opts.native_tools,
                    self.thread_id,
                    self.egress,
                )?;
            let concrete = if concrete.trim().is_empty() {
                pin
            } else {
                &concrete
            };
            return Ok((
                model,
                ResolvedModelRoute::new(
                    PROVIDER_OPENHUMAN,
                    concrete,
                    Self::route_name(request, role),
                ),
            ));
        }

        let resolved = self.ctx.resolved();
        let context_extensions = resolved
            .as_deref()
            .map(crate::core::runtime::CoreContext::extensions);
        let mut parsed_route = ProviderRoute::parse(&provider);
        if parsed_route.is_none()
            && provider
                .trim()
                .eq_ignore_ascii_case(CLAUDE_AGENT_SDK_PROVIDER)
        {
            parsed_route = Some(ProviderRoute {
                provider: CLAUDE_AGENT_SDK_PROVIDER.to_owned(),
                model: self.config.claude_agent_sdk.default_model.clone(),
                temperature: None,
            });
        }
        if let Some(route) = parsed_route.as_mut() {
            if !pin.is_empty()
                && matches!(
                    route.kind(),
                    RouteKind::ClaudeCode | RouteKind::ClaudeAgentSdk
                )
            {
                route.model = pin.to_owned();
            }
        }
        if let Some(route) = parsed_route.as_ref().filter(|route| {
            matches!(
                route.kind(),
                RouteKind::ClaudeCode | RouteKind::ClaudeAgentSdk
            )
        }) {
            if route.temperature.is_some() && matches!(route.kind(), RouteKind::ClaudeCode) {
                log::warn!(
                    "[providers][chat-factory] claude-code provider: per-model temperature override \
                     is accepted but not wired through to the CLI — the @<temp> suffix is ignored"
                );
            }
            let effective_model = route.model.trim();
            anyhow::ensure!(
                !effective_model.is_empty(),
                "subprocess model route for role '{role}' has an empty model"
            );
            let route_kind = route.kind();
            let egress_name = match route_kind {
                RouteKind::ClaudeCode => format!(
                    "{}{}",
                    tinyagents_harness::providers::claude_code::PROVIDER_PREFIX,
                    effective_model
                ),
                RouteKind::ClaudeAgentSdk => format!("{CLAUDE_AGENT_SDK_PREFIX}{effective_model}"),
                _ => unreachable!("subprocess route was filtered"),
            };
            emit_inference_egress(role, &egress_name, self.egress);
            let workspace = self
                .config
                .config_path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| self.config.workspace_dir.clone());
            let documents = crate::storage::current_scoped()?
                .ok_or_else(|| anyhow::anyhow!("storage is unavailable for Claude provider state"))?
                .documents()
                .clone();
            let context = tinyagents_harness::host::model_factory::SubprocessProviderContext {
                workspace_dir: workspace,
                action_dir: self.config.action_dir.clone(),
                documents,
                claude_agent_sdk: (&self.config.claude_agent_sdk).into(),
                mcp_provider: Some(Arc::new(OpenHumanMcpEndpoint)),
            };
            let model = tinyagents_harness::host::model_factory::build_claude_subprocess_model(
                route, &context,
            )?
            .ok_or_else(|| anyhow::anyhow!("unsupported subprocess route"))?;
            let route_provider = route.provider.clone();
            let resolved_model = effective_model.to_owned();
            let route = ResolvedModelRoute::new(
                route_provider,
                resolved_model,
                Self::route_name(request, role),
            );
            return Ok((model, route));
        }

        let (model, route_provider, resolved_model) = match parsed_route
            .as_ref()
            .map(ProviderRoute::kind)
        {
            Some(RouteKind::Local(_)) | None
                if tinyinference_local::profile::kind_from_provider_string(&provider).is_some() =>
            {
                let local = try_create_local_runtime_chat_model_from_string(
                    role,
                    &provider,
                    self.config,
                    self.egress,
                );
                let (model, resolved) = match local {
                    Some(result) => result?,
                    None => {
                        return Err(unresolved_chat_model_error_for_access(
                            role,
                            &provider,
                            self.config,
                            self.egress,
                            context_extensions,
                            CredentialAccess::Runtime(&self.credentials),
                        ));
                    }
                };
                let provider = model
                    .profile()
                    .and_then(|p| p.provider.clone())
                    .unwrap_or_else(|| "local".into());
                (model, provider, resolved)
            }
            Some(RouteKind::Cloud { .. }) => {
                let cloud = super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
                        role,
                        &provider,
                        self.config,
                        request.opts.native_tools,
                        self.egress,
                        context_extensions,
                        CredentialAccess::Runtime(&self.credentials),
                        None,
                        None,
                )
                .await;
                let (model, resolved) = match cloud {
                    Some(result) => result?,
                    None => {
                        return Err(unresolved_chat_model_error_for_access(
                            role,
                            &provider,
                            self.config,
                            self.egress,
                            context_extensions,
                            CredentialAccess::Runtime(&self.credentials),
                        ));
                    }
                };
                let route_provider = model
                    .profile()
                    .and_then(|p| p.provider.clone())
                    .unwrap_or_else(|| {
                        parsed_route
                            .as_ref()
                            .expect("parsed cloud route")
                            .provider
                            .clone()
                    });
                (model, route_provider, resolved)
            }
            _ => {
                return Err(unresolved_chat_model_error_for_access(
                    role,
                    &provider,
                    self.config,
                    self.egress,
                    context_extensions,
                    CredentialAccess::Runtime(&self.credentials),
                ));
            }
        };

        let route = ResolvedModelRoute::new(
            route_provider,
            resolved_model,
            Self::route_name(request, role),
        );
        Ok((model, route))
    }
}

async fn build_one_shot(
    ctx: &DispatchCtx,
    role: &str,
    provider: Option<&str>,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let egress = EgressLedger::none();
    let credentials = super::credentials::RuntimeCredentialSnapshot::resolve(config).await;
    let factory = OpenHumanModelFactory {
        ctx,
        config,
        primary_provider: provider,
        egress: &egress,
        thread_id: None,
        attach_host_attachments: false,
        credentials,
        host_controls: None,
        summarizer_model: None,
    };
    let request = ModelBuildRequest {
        role: role.to_owned(),
        route: None,
        purpose: ModelPurpose::Primary,
        model_pin: None,
        opts: BuildOpts {
            native_tools: true,
            temperature: None,
            egress_tag: None,
        },
    };
    factory.check(&request)?;
    let (model, route) = factory.build(&request).await?;
    Ok((model, route.model))
}

#[async_trait::async_trait]
impl ModelFactory<()> for OpenHumanModelFactory<'_> {
    async fn build(
        &self,
        request: &ModelBuildRequest,
    ) -> tinyagents_harness::Result<(Arc<dyn ChatModel<()>>, ResolvedModelRoute)> {
        let (model, route) = self
            .build_request(request)
            .await
            .map_err(|error| tinyagents_harness::TinyAgentsError::Model(error.to_string()))?;
        let opts = ModelRequestDefaults {
            model: (!route.model.is_empty()).then(|| route.model.clone()),
            temperature: request.opts.temperature,
        };
        let mut model = with_request_defaults(model, opts);
        if self.attach_host_attachments {
            model = if self.injected(request).is_some() {
                crate::attachments::provider::wrap_injected_controlled(
                    self.ctx,
                    model,
                    Arc::new(self.config.clone()),
                    self.host_controls,
                )
            } else {
                crate::attachments::provider::wrap_controlled(
                    self.ctx,
                    model,
                    Arc::new(self.config.clone()),
                    &route.model,
                    &route.provider,
                    self.host_controls,
                )
                .await
            };
        }
        Ok((model, route))
    }
}

fn with_request_defaults(
    model: Arc<dyn ChatModel<()>>,
    defaults: ModelRequestDefaults,
) -> Arc<dyn ChatModel<()>> {
    Arc::new(ProfileOverrideModel::new_preserving_profile(model).with_defaults(defaults))
}

/// Build an `Arc<dyn ChatModel>` for the given workload role, under `ctx`.
///
/// The crate [`ChatModel`] is the model interface for the harness and one-shot
/// inference callers. Production and tests both inject this native interface;
/// `temperature` is applied as the request default while an explicit per-call
/// value still wins.
pub async fn create_chat_model_in(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    Ok(
        create_chat_model_with_model_id_in(ctx, role, config, temperature)
            .await?
            .0,
    )
}

/// Like [`create_chat_model_in`], but also returns the resolved model id.
///
/// One-shot callers that persist or log the concrete model (e.g. the memory
/// summarise audit) need the id the role resolved to; the plain
/// [`create_chat_model_in`] drops it. The host model overrides of the agent
/// `ctx` runs for win over the configured route.
pub async fn create_chat_model_with_model_id_in(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let (model, model_id) = create_chat_model_with_model_id_inner(ctx, role, config).await?;
    Ok((with_default_temperature(model, temperature), model_id))
}

pub(super) async fn create_chat_model_with_model_id_inner(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    build_one_shot(ctx, role, None, config).await
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
///    OpenHuman backend, the registered provider's readiness probe makes one
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
pub async fn probe_inference_readiness(
    ctx: &DispatchCtx,
    role: &str,
    config: &Config,
) -> Result<(), String> {
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    if test_provider_override::current().is_some() {
        log::debug!(
            "[flows][inference-probe] role={role} test model override active — skipping probe"
        );
        return Ok(());
    }

    log::debug!("[flows][inference-probe] role={role} verifying model construction");
    if let Err(e) = create_chat_model_with_model_id_inner(ctx, role, config).await {
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
    let resolved_context = ctx.resolved();
    let extensions = resolved_context
        .as_deref()
        .map(crate::core::runtime::CoreContext::extensions);
    let Some(entry) = crate::inference::registry::chat_model_for(extensions, PROVIDER_OPENHUMAN)
    else {
        log::debug!("[flows][inference-probe] role={role} managed provider was unregistered");
        return Ok(());
    };
    let Some(probe) = entry.probe else {
        // The registered provider has no readiness probe: construction above
        // succeeded, and there is nothing further to ask it.
        log::debug!("[flows][inference-probe] role={role} managed provider registered no probe");
        return Ok(());
    };
    let provider_context = crate::inference::registry::ProviderContext::new(
        config,
        &crate::security::egress::EgressLedger::none(),
    );
    let model_id = entry
        .models
        .as_ref()
        .and_then(|models| (models.resolve_model)(role, &provider_context, None))
        .or_else(|| {
            config
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| format!("provider {PROVIDER_OPENHUMAN} has no model for role {role}"))?;
    let result = probe(tinyinference_llm::providers::overrides::ReadinessRequest {
        role: role.to_string(),
        model: model_id.clone(),
        context: provider_context,
    })
    .await;
    log::debug!(
        "[flows][inference-probe] role={role} model={model_id} probe result: {}",
        if result.is_ok() { "ready" } else { "not ready" }
    );
    result
}

/// Build an `Arc<dyn ChatModel>` from an explicit provider string and config.
///
/// The explicit-string counterpart of [`create_chat_model`].
pub async fn create_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    create_chat_model_from_string_with_model_id(role, provider, config, temperature)
        .await
        .map(|(model, _)| model)
}

/// Build a crate [`ChatModel`] from an explicit provider string and return the
/// concrete model id selected by that provider.
///
/// Managed, local-runtime, configured cloud-slug, Claude SDK/Code, and Codex
/// strings all construct native `ChatModel` implementations directly.
pub async fn create_chat_model_from_string_with_model_id(
    role: &str,
    provider: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let (model, model_id) =
        create_chat_model_from_string_with_model_id_inner(role, provider, config).await?;
    Ok((with_default_temperature(model, temperature), model_id))
}

pub(super) async fn create_chat_model_from_string_with_model_id_inner(
    role: &str,
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let resolved_provider;
    let provider = if provider.trim().is_empty() || provider.trim() == "cloud" {
        resolved_provider = resolve_primary_cloud_provider_string(config);
        resolved_provider.as_str()
    } else {
        provider
    };
    build_one_shot(&DispatchCtx::ambient(), role, Some(provider), config).await
}

pub(super) fn with_default_temperature(
    model: Arc<dyn ChatModel<()>>,
    temperature: f64,
) -> Arc<dyn ChatModel<()>> {
    with_request_defaults(
        model,
        ModelRequestDefaults {
            model: None,
            temperature: Some(temperature),
        },
    )
}

/// Reproduce the legacy provider factory's access gates and diagnostics for a
/// provider string that none of the crate-native model constructors accepted.
///
/// Successful production routes never reach this function. Keeping error
/// resolution separate means `create_chat_model*` no longer constructs a
/// legacy `Provider` merely to discover that a route is invalid.
pub(super) fn unresolved_chat_model_error_for_access(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
    extensions: Option<&crate::core::slots::Extensions>,
    credential_access: CredentialAccess<'_>,
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
        return unresolved_chat_model_error_for_access(
            role,
            &resolve_primary_cloud_provider_string(config),
            config,
            egress,
            extensions,
            credential_access,
        );
    }

    #[cfg(not(test))]
    if let CredentialAccess::Runtime(snapshot) = credential_access {
        if let Err(error) =
            super::access_gates::verify_session_active_with_snapshot(config, snapshot)
        {
            return error;
        }
    }

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
        let result = match credential_access {
            CredentialAccess::Runtime(snapshot) => resolve_cloud_slug_with_access(
                role,
                slug.trim(),
                &model,
                config,
                extensions,
                CredentialAccess::Runtime(snapshot),
                None,
            ),
            CredentialAccess::Inspection => resolve_cloud_slug_with_access(
                role,
                slug.trim(),
                &model,
                config,
                extensions,
                CredentialAccess::Inspection,
                None,
            ),
        };
        return match result {
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
