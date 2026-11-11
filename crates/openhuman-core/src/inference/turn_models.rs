//! [`TurnModels`] and [`TurnModelSource`] — the per-turn crate-native model
//! bundle and the model-agnostic handle that builds it.

use std::sync::Arc;

use crate::core::runtime::DispatchCtx;

use tinyinference_llm::model::ProfileOverrideModel;

/// The per-turn crate [`ChatModel`](tinyinference_llm::model::ChatModel) set,
/// assembled by TinyAgents' `TurnModelSet` for one turn.
///
/// [`OpenHumanTurnPlanner`](crate::agent::tinyagents::turn_plan::OpenHumanTurnPlanner) resolves calls through its native resolver.
pub(crate) struct TurnModels {
    /// The turn's effective/primary model (registry default + dispatch target).
    pub(crate) primary: Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    /// Additive workload-tier routes (registry name → model), excluding the
    /// primary; the crate registry resolves fallback/selection across them.
    pub(crate) routes: Vec<(String, Arc<dyn tinyinference_llm::model::ChatModel<()>>)>,
    /// A model for the context-window summarizer (a distinct adapter instance so
    /// its provider errors don't touch the turn's `error_slot`). On a
    /// config-built source it calls `[context] summarizer_model` on the turn's
    /// provider route when that is set, else the turn's own model.
    pub(crate) summarizer: Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    /// The model id [`summarizer`](Self::summarizer) calls, as the compaction
    /// summarizer reports it in its logs and summary provenance.
    pub(crate) summarizer_model_id: String,
    /// How the free-form summarizer renders tool activity and caps its output
    /// (`[context] summarizer_tool_rendering` / `summarizer_max_output_tokens`
    /// on a config-built source; the defaults on an injected one).
    pub(crate) summarizer_settings: crate::config::SummarizerSettings,
    /// Recovers the primary's original (downcastable) provider error on failure.
    pub(crate) error_slot: tinyinference_llm::model::ModelErrorSlot,
    /// Native owner set retained so the resolver keeps its route and primary
    /// error slots.
    pub(crate) native_model_set: Option<tinyagents_harness::host::TurnModelSet<()>>,
    /// Set when every model here renders attachments above a control slot;
    /// the turn's spend controls are installed there instead of on top.
    pub(crate) host_controls: Option<crate::inference::host_controls::HostControlSlot>,
    /// Provider telemetry id (`{provider_id}.{model}` in Langfuse), captured from
    /// the source `Provider` at build time. Carried here (issue #4249, Phase 3 /
    /// Motion A) so the harness turn path no longer reads it off a raw
    /// `Provider` — the harness holds crate model types only.
    provider_id: String,
    /// The primary model's effective context window (drives the context-window
    /// summarization step). Resolved by the producer/factory before build so the
    /// harness graph no longer makes the async `effective_context_window` call.
    context_window: Option<u64>,
    /// Whether the source provider does native tool-calling — the harness uses
    /// this only to pick the history-suffix dispatcher (native envelope vs
    /// prompt-guided text). Captured from the provider at build time.
    native_tools: bool,
    /// Whether the source provider is vision-capable — the harness uses this to
    /// gate multimodal placeholder rehydration. Captured at build time.
    supports_vision: bool,
    /// Whether the source provider is local / self-hosted (Ollama, LM Studio,
    /// ...). Selects the longer wall-clock ceilings (#6042).
    is_local: bool,
}

impl TurnModels {
    /// Bind the already selected turn routes to the harness-owned resolver.
    pub(crate) fn resolver(&self) -> tinyagents_harness::host::PrebuiltModelResolver<()> {
        if let Some(models) = &self.native_model_set {
            return models.resolver();
        }
        tinyagents_harness::host::PrebuiltModelResolver::new(
            self.primary.clone(),
            self.routes.to_vec(),
        )
        .with_lead_model(Arc::new(tinyinference_llm::model::ErrorSlotModel::new(
            self.primary.clone(),
            self.error_slot.clone(),
        )))
    }
    /// Whether the primary provider is local / self-hosted.
    pub(crate) fn is_local(&self) -> bool {
        self.is_local
    }

    /// Adds a tier route to a test bundle (the injected-model builder has none).
    #[cfg(test)]
    pub(crate) fn with_test_route(
        mut self,
        name: &str,
        model: Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    ) -> Self {
        self.routes.push((name.to_string(), model));
        self
    }

    /// The model id the context-compaction summarizer calls.
    pub(crate) fn summarizer_model_id(&self) -> &str {
        &self.summarizer_model_id
    }

    /// Provider telemetry id for this turn (`{provider_id}.{model}`).
    pub(crate) fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The primary model's effective context window, if known.
    pub(crate) fn context_window(&self) -> Option<u64> {
        self.context_window
    }

    /// Whether the source provider does native tool-calling.
    pub(crate) fn native_tools(&self) -> bool {
        self.native_tools
    }

    /// Whether the source provider is vision-capable.
    pub(crate) fn supports_vision(&self) -> bool {
        self.supports_vision
    }
}

/// The compaction summarizer model `[context] summarizer_model` names, when it
/// names one: a blank value is unset.
fn configured_summarizer_model(config: &crate::config::Config) -> Option<&str> {
    config
        .context
        .summarizer_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

/// The model id this turn's compaction summarizer calls: the configured one,
/// else the turn's own. Logs the choice once per turn build.
fn choose_summarizer_model(configured: Option<&str>, turn_model: &str) -> String {
    let (model, source) = match configured {
        Some(model) => (model, "config"),
        None => (turn_model, "turn"),
    };
    tracing::debug!("[context_compression] summarizer model={model} source={source}");
    model.to_owned()
}

/// Build one per-turn set through TinyAgents' model-factory contract.
///
/// [`OpenHumanTurnPlanner`](crate::agent::tinyagents::turn_plan::OpenHumanTurnPlanner) receives the native route resolver. Provider metadata
/// (`provider_id` / `native_tools` / `supports_vision`) is derived by the caller
/// ([`TurnModelSource::build`]) from the resolved provider string and config.
/// `error_slot` is a fresh
/// empty slot — crate-native models surface `TinyAgentsError` directly (no
/// downcastable `anyhow` to preserve), so typed provider-error *recovery* is unused
/// here (Sentry suppression is unaffected — both `skips_sentry` cases are raised in
/// the host turn loop).
#[allow(clippy::too_many_arguments)]
async fn build_turn_models_crate(
    ctx: &DispatchCtx,
    role: &str,
    config: &crate::config::Config,
    model: &str,
    temperature: f64,
    context_window: Option<u64>,
    primary_override: Option<&str>,
    provider_id: String,
    native_tools: bool,
    _supports_vision: bool,
    is_local: bool,
    thread_id: Option<&str>,
) -> anyhow::Result<TurnModels> {
    let egress = crate::security::egress::EgressLedger::per_turn();
    let credentials =
        crate::inference::provider::factory::RuntimeCredentialSnapshot::resolve(config).await;
    let configured_summarizer = configured_summarizer_model(config);
    let host_controls = crate::inference::host_controls::HostControlSlot::default();
    let factory = crate::inference::provider::factory::OpenHumanModelFactory {
        ctx,
        config,
        primary_provider: primary_override,
        egress: &egress,
        thread_id,
        attach_host_attachments: true,
        credentials,
        host_controls: Some(&host_controls),
        summarizer_model: configured_summarizer,
    };
    let opts = tinyagents_harness::host::BuildOpts {
        native_tools: true,
        temperature: Some(temperature),
        egress_tag: None,
    };
    let resolved = ctx.resolved();
    let extensions = resolved
        .as_deref()
        .map(crate::core::runtime::CoreContext::extensions);
    let configured_provider = primary_override
        .map(str::to_owned)
        .unwrap_or_else(|| crate::inference::provider::provider_for_role(role, config));
    let provider_route = tinyinference_providers::route::ProviderRoute::parse(&configured_provider)
        .map(|route| route.provider)
        .unwrap_or_else(|| configured_provider.clone());
    let primary_is_injected = ctx
        .host_overrides()
        .and_then(|overrides| overrides.model_for(role))
        .is_some();
    #[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
    let primary_is_injected = primary_is_injected
        || crate::inference::provider::factory::test_provider_override::current().is_some();
    let workload_requests = tinyinference_llm::model::workload::WORKLOAD_ROUTE_TIERS
        .iter()
        .filter_map(|route| {
            let role = crate::inference::provider_model::role_for_provider_model_for(
                extensions,
                &provider_route,
                route,
            );
            if primary_is_injected
                && !ctx.host_overrides().is_some_and(|overrides| {
                    overrides.role_models.contains_key(*route)
                        || overrides.role_models.contains_key(role)
                })
            {
                return None;
            }
            Some(tinyagents_harness::host::ModelBuildRequest {
                role: role.to_owned(),
                route: Some((*route).to_owned()),
                purpose: tinyagents_harness::host::ModelPurpose::WorkloadTier((*route).to_owned()),
                model_pin: Some((*route).to_owned()),
                opts: opts.clone(),
            })
        })
        .collect();
    let summarizer_model_id = choose_summarizer_model(configured_summarizer, model);
    let built = tinyagents_harness::host::TurnModelSet::<()>::builder(&factory, &factory, role)
        .primary_model_pin(model)
        .primary_opts(opts.clone())
        .workload_requests(workload_requests)
        .summarization(role, Some(summarizer_model_id.clone()))
        .summarization_opts(opts)
        .build()
        .await?;
    let mut registrations = built.registrations("primary").into_iter();
    let primary = registrations.next().expect("turn set contains a primary").1;
    let routes = registrations.collect();
    let summarizer = built.summarizer();
    let error_slot = built.primary_error_slot();

    let supports_vision = primary.profile().is_some_and(|p| p.modalities.image_in);
    Ok(TurnModels {
        primary,
        routes,
        summarizer,
        summarizer_model_id,
        summarizer_settings: config.context.summarizer_settings(),
        error_slot,
        native_model_set: Some(built),
        host_controls: Some(host_controls),
        provider_id,
        context_window,
        native_tools,
        supports_vision,
        is_local,
    })
}

/// A model-agnostic source of per-turn [`TurnModels`] — the seam-owned handle the
/// agent harness holds instead of a provider-specific client (issue #4249, Phase 3
/// / Motion A).
///
/// An [`Agent`](crate::agent::OpenHumanSessionHost) (and each channel/subagent turn
/// request) is model-agnostic: it holds this source and builds a *tiered* crate
/// [`ChatModel`] set (primary + workload-tier fallback routes + summarizer) per
/// turn. Production sources retain only crate-native role/config metadata;
/// provider-backed sources remain for injected tests and bespoke clients. Constructed in
/// exactly one place — [`create_turn_model_source`](crate::inference::provider::factory::create_turn_model_source).
#[derive(Clone)]
pub struct TurnModelSource {
    /// A directly injected crate model. This is the replacement test seam for
    /// provider-backed mocks while WP-1 removes `native model adapter`.
    pub(crate) direct_model: Option<Arc<dyn tinyinference_llm::model::ChatModel<()>>>,
    /// Acting-workspace config for injected models that resolve attachments.
    attachment_config: Option<Arc<crate::config::Config>>,
    /// When set, [`build`](Self::build) / [`build_summarizer`](Self::build_summarizer)
    /// construct **crate-native** models from `(role, config)` (Phase 3 P3-B) via
    /// [`build_turn_models_crate`]. Crate-native sources keep `provider` as
    /// `None`; build failures propagate instead of falling back to the host wire
    /// client.
    pub(crate) crate_native: Option<CrateNativeSource>,
}

/// The `(role, config)` a crate-native [`TurnModelSource`] builds its tiered
/// [`TurnModels`] from per turn.
#[derive(Clone)]
pub(crate) struct CrateNativeSource {
    role: String,
    config: Arc<crate::config::Config>,
    /// Built-under context (weak: it is the tenant lease); supplies host model overrides.
    core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
    /// An explicit provider string for the **primary** model, overriding the
    /// role's default resolution. Set when a producer's effective provider differs
    /// from `provider_for_role(role)` — e.g. triage's #1257 force-managed override
    /// (`build_remote_provider`). `None` builds the primary from `role`. Routes
    /// always use the standard workload tiers.
    primary_override: Option<String>,
}

impl CrateNativeSource {
    fn ctx(&self) -> DispatchCtx {
        DispatchCtx::from_weak(self.core.as_ref())
    }
}

impl TurnModelSource {
    /// The context this source builds under: its own, else the ambient one.
    pub(crate) fn dispatch_ctx(&self) -> DispatchCtx {
        (self.crate_native.as_ref()).map_or_else(DispatchCtx::ambient, CrateNativeSource::ctx)
    }

    /// Whether the selected primary accepts native tool calls, without building
    /// models or publishing the per-turn egress disclosures that model
    /// construction owns. Session assembly needs this before the actual turn
    /// is built; using `build` here would create a second per-turn ledger.
    pub(crate) fn supports_native_tool_calling(&self) -> bool {
        if let Some(source) = &self.crate_native {
            if let Some(custom) = source
                .ctx()
                .host_overrides()
                .and_then(|local| local.model_for(&source.role))
            {
                return custom.profile().cloned().unwrap_or_default().tool_calling;
            }
        }
        if let Some(direct) = &self.direct_model {
            return direct.profile().cloned().unwrap_or_default().tool_calling;
        }
        if let Some(source) = &self.crate_native {
            let provider_string = source
                .primary_override
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    crate::inference::provider::provider_for_role(&source.role, &source.config)
                });
            return !tinyinference_local::profile::is_local_provider_string(&provider_string);
        }
        false
    }

    /// Use an already-constructed TinyAgents model as the complete turn model
    /// source. Intended for deterministic tests and embedding callers that do
    /// not need role/config-based route construction.
    pub fn from_model(model: Arc<dyn tinyinference_llm::model::ChatModel<()>>) -> Self {
        Self {
            direct_model: Some(model),
            attachment_config: None,
            crate_native: None,
        }
    }

    /// Bind acting-workspace attachment policy to an injected model source.
    /// Factory-backed sources already carry their own scoped config.
    pub(crate) fn with_attachment_config(mut self, config: Arc<crate::config::Config>) -> Self {
        self.attachment_config = Some(config);
        self
    }

    /// Inject a model while supplying capability metadata that the model itself
    /// does not expose (common for deterministic scripted tests).
    pub(crate) fn from_model_with_profile(
        model: Arc<dyn tinyinference_llm::model::ChatModel<()>>,
        profile: tinyinference_llm::model::ModelProfile,
    ) -> Self {
        Self::from_model(Arc::new(ProfileOverrideModel::new(model, profile)))
    }

    /// Build a crate-native source: [`build`](Self::build) uses TinyAgents' `TurnModelSet`
    /// to construct the turn's native models under the host policy adapter. Used by the session-builder producer
    /// (`crate_native_provider`); the triage path uses
    /// [`new_crate_native_from_string`](Self::new_crate_native_from_string).
    pub(crate) fn new_crate_native(
        ctx: &DispatchCtx,
        role: impl Into<String>,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            direct_model: None,
            attachment_config: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                core: ctx.downgrade(),
                primary_override: None,
            }),
        }
    }

    /// Build a crate-native source whose **primary** model uses an explicit
    /// `provider_string` rather
    /// than the role's default resolution — the triage path's #1257 force-managed
    /// override (`build_remote_provider` picks the effective string). Routes still
    /// use the standard workload tiers.
    pub(crate) fn new_crate_native_from_string(
        ctx: &DispatchCtx,
        role: impl Into<String>,
        provider_string: impl Into<String>,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            direct_model: None,
            attachment_config: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                core: ctx.downgrade(),
                primary_override: Some(provider_string.into()),
            }),
        }
    }

    /// Resolve the model's effective context window — the value that drives the
    /// context-window summarization step. Resolved before [`build`](Self::build)
    /// so the harness graph makes no async call.
    ///
    /// A crate-native source asks the provider: config override, then the
    /// provider's own model listing (bounded and cached, corrected by any limit
    /// an overflow error stated), and only then the static tables
    /// ([`crate::inference::context_window`]).
    pub(crate) async fn effective_context_window(&self, model: &str) -> Option<u64> {
        if let Some(direct) = &self.direct_model {
            return direct
                .profile()
                .and_then(|profile| profile.max_input_tokens);
        }
        let Some(source) = self.crate_native.as_ref() else {
            return crate::inference::model_context::context_window_for_model(model);
        };
        if let Some(custom) = source
            .ctx()
            .host_overrides()
            .and_then(|local| local.model_for(&source.role))
        {
            return custom
                .profile()
                .and_then(|profile| profile.max_input_tokens);
        }
        let provider_string = source.primary_override.clone().unwrap_or_else(|| {
            crate::inference::provider::provider_for_role(&source.role, &source.config)
        });
        let resolved_context = source.ctx().resolved();
        let extensions = resolved_context
            .as_deref()
            .map(crate::core::runtime::CoreContext::extensions);
        crate::inference::context_window::resolve_context_window_in(
            &source.role,
            &provider_string,
            model,
            &source.config,
            extensions,
        )
        .await
    }

    /// Build this turn's [`TurnModels`] (primary + tier routes + summarizer),
    /// capturing provider telemetry id + capabilities onto the bundle.
    pub(crate) async fn build(
        &self,
        model: &str,
        temperature: f64,
        context_window: Option<u64>,
        thread_id: Option<&str>,
    ) -> anyhow::Result<TurnModels> {
        if let Some(direct) = &self.direct_model {
            let profile = direct.profile().cloned();
            let provider_id = profile
                .as_ref()
                .and_then(|profile| profile.provider.clone())
                .clone()
                .unwrap_or_else(|| "injected".to_string());
            let native_tools = profile.as_ref().is_some_and(|profile| profile.tool_calling);
            let supports_vision = profile
                .as_ref()
                .is_some_and(|profile| profile.modalities.image_in);
            let is_local =
                crate::agent::tinyagents::turn_policy::provider_is_self_hosted(&provider_id, None);
            let context_window = context_window.or_else(|| {
                profile
                    .as_ref()
                    .and_then(|profile| profile.max_input_tokens)
            });
            let primary: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
                if let Some(window) = context_window.filter(|window| *window > 0) {
                    let mut profile = profile.unwrap_or_default();
                    profile.max_input_tokens = Some(window);
                    Arc::new(
                        ProfileOverrideModel::new(direct.clone(), profile)
                            .with_request_model(model)
                            .with_request_temperature(temperature),
                    )
                } else {
                    Arc::new(
                        ProfileOverrideModel::new_preserving_profile(direct.clone())
                            .with_request_model(model)
                            .with_request_temperature(temperature),
                    )
                };
            let host_controls = self
                .attachment_config
                .as_ref()
                .map(|_| crate::inference::host_controls::HostControlSlot::default());
            let (primary, summarizer) = match &self.attachment_config {
                Some(config) => (
                    crate::attachments::provider::wrap_injected_controlled(
                        &self.dispatch_ctx(),
                        primary,
                        config.clone(),
                        host_controls.as_ref(),
                    ),
                    crate::attachments::provider::wrap_injected_controlled(
                        &self.dispatch_ctx(),
                        direct.clone(),
                        config.clone(),
                        host_controls.as_ref(),
                    ),
                ),
                None => (primary, direct.clone()),
            };
            return Ok(TurnModels {
                primary,
                routes: Vec::new(),
                summarizer,
                // An injected model is the whole source; there is no route
                // to call another model on.
                summarizer_model_id: choose_summarizer_model(None, model),
                summarizer_settings: Default::default(),
                error_slot: Arc::new(std::sync::Mutex::new(None)),
                native_model_set: None,
                host_controls,
                provider_id,
                context_window,
                native_tools,
                supports_vision,
                is_local,
            });
        }
        if let Some(cn) = &self.crate_native {
            let ctx = cn.ctx();
            let host_injected = ctx
                .host_overrides()
                .and_then(|overrides| overrides.model_for(&cn.role));
            let injected_profile = host_injected.as_ref().and_then(|model| model.profile());
            let provider_string = cn.primary_override.clone().unwrap_or_else(|| {
                crate::inference::provider::provider_for_role(&cn.role, &cn.config)
            });
            // Tool/vision gating keeps its historical provider-name rule; only
            // the wall-clock ceilings use endpoint-aware self-hosted detection.
            let (
                provider_id,
                native_tools,
                supports_vision,
                _is_local,
                self_hosted,
                context_window,
            ) = if let Some(profile) = injected_profile {
                let provider_id = profile
                    .provider
                    .as_deref()
                    .unwrap_or("injected")
                    .to_string();
                (
                    provider_id.clone(),
                    profile.tool_calling,
                    profile.modalities.image_in,
                    crate::agent::tinyagents::turn_policy::provider_is_self_hosted(
                        &provider_id,
                        None,
                    ),
                    crate::agent::tinyagents::turn_policy::provider_is_self_hosted(
                        &provider_id,
                        None,
                    ),
                    context_window.or(profile.max_input_tokens),
                )
            } else if host_injected.is_some() {
                // A host-injected model owns the provider choice even when
                // it does not publish a profile. Keep capability claims
                // conservative and retain only the caller's window.
                (
                    "injected".to_string(),
                    false,
                    false,
                    false,
                    false,
                    context_window,
                )
            } else {
                let is_local =
                    tinyinference_local::profile::is_local_provider_string(&provider_string);
                let self_hosted = crate::agent::tinyagents::turn_policy::provider_is_self_hosted(
                    &provider_string,
                    crate::agent::tinyagents::turn_policy::local_openai_endpoint(&cn.config)
                        .as_deref(),
                );
                let provider_id = if provider_string == "openhuman"
                    || provider_string.is_empty()
                    || provider_string == "cloud"
                {
                    "managed".to_string()
                } else {
                    provider_string
                        .split(':')
                        .next()
                        .unwrap_or(&provider_string)
                        .to_string()
                };
                (
                    provider_id,
                    !is_local,
                    !is_local,
                    is_local,
                    self_hosted,
                    context_window,
                )
            };
            return build_turn_models_crate(
                &ctx,
                &cn.role,
                &cn.config,
                model,
                temperature,
                context_window,
                cn.primary_override.as_deref(),
                provider_id,
                native_tools,
                supports_vision,
                self_hosted,
                thread_id,
            )
            .await;
        }
        Err(anyhow::anyhow!("turn model source is missing a model"))
    }

    /// Build a standalone summarizer [`ChatModel`](tinyinference_llm::model::ChatModel)
    /// over this source's provider — a fresh adapter (own error slot) for one-off
    /// summary calls outside the main turn (e.g. the sub-agent cap-hit checkpoint),
    /// so the caller can `invoke` without naming the `Provider` trait. The output
    /// cap rides the per-call `ModelRequest`, not the model.
    pub(crate) async fn build_summarizer(
        &self,
        model: &str,
        temperature: f64,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Arc<dyn tinyinference_llm::model::ChatModel<()>>> {
        if let Some(direct) = &self.direct_model {
            let summarizer: Arc<dyn tinyinference_llm::model::ChatModel<()>> = Arc::new(
                ProfileOverrideModel::new_preserving_profile(direct.clone())
                    .with_request_model(model)
                    .with_request_temperature(temperature),
            );
            return Ok(match &self.attachment_config {
                Some(config) => crate::attachments::provider::wrap_injected(
                    &self.dispatch_ctx(),
                    summarizer,
                    config.clone(),
                ),
                None => summarizer,
            });
        }
        if let Some(cn) = &self.crate_native {
            if let Some(custom) = cn.ctx().host_overrides().and_then(|local| {
                local
                    .model_for("summarization")
                    .or_else(|| local.model_for(&cn.role))
            }) {
                return Ok(crate::attachments::provider::wrap_injected(
                    &cn.ctx(),
                    custom,
                    cn.config.clone(),
                ));
            }
            let ctx = cn.ctx();
            let egress = crate::security::egress::EgressLedger::per_turn();
            let factory = crate::inference::provider::factory::OpenHumanModelFactory {
                ctx: &ctx,
                config: &cn.config,
                primary_provider: cn.primary_override.as_deref(),
                egress: &egress,
                thread_id,
                attach_host_attachments: true,
                credentials:
                    crate::inference::provider::factory::RuntimeCredentialSnapshot::resolve(
                        &cn.config,
                    )
                    .await,
                host_controls: None,
                summarizer_model: None,
            };
            let opts = tinyagents_harness::host::BuildOpts {
                native_tools: true,
                temperature: Some(temperature),
                egress_tag: None,
            };
            let request = tinyagents_harness::host::ModelBuildRequest {
                role: cn.role.clone(),
                route: None,
                purpose: tinyagents_harness::host::ModelPurpose::Summarization,
                model_pin: Some(model.to_owned()),
                opts,
            };
            tinyagents_harness::host::ModelBuildGate::check(&factory, &request)?;
            let (model, _) =
                tinyagents_harness::host::ModelFactory::build(&factory, &request).await?;
            return Ok(model);
        }
        Err(anyhow::anyhow!("turn model source is missing a model"))
    }
}

#[cfg(test)]
#[path = "turn_models_tests.rs"]
mod tests;
