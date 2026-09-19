//! [`TurnModels`] and [`TurnModelSource`] — the per-turn crate-native model
//! bundle and the model-agnostic handle that builds it.

use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::tinyagents::model::{
    BuiltTurnModels, ProfileOverrideModel, TierRoutes, TurnChatModel,
};
use crate::agent::tinyagents::routes;
use tinyagents_harness::host::{ModelResolveRequest, ModelResolver};
use tinyinference_llm::model::{ResolvedModelRoute, RouteRecordingModel};

pub(crate) fn tinyagents_depth_error(
    err: &tinyagents_harness::TinyAgentsError,
) -> Option<crate::agent::harness::subagent_runner::SubagentRunError> {
    match err {
        tinyagents_harness::TinyAgentsError::SubAgentDepth(max_depth)
        | tinyagents_harness::TinyAgentsError::RecursionLimit(max_depth) => Some(
            crate::agent::harness::subagent_runner::SubagentRunError::SpawnDepthExceeded {
                attempted_depth: max_depth.saturating_add(1),
                max_depth: *max_depth,
            },
        ),
        _ => None,
    }
}

/// The per-turn crate [`ChatModel`](tinyinference_llm::model::ChatModel) set,
/// built once from an openhuman [`Provider`] by [`build_turn_models`] — the
/// single place a turn's `native model adapters are constructed (issue #4249, Phase 5).
///
/// [`assemble_turn_harness`](super::harness_assembly::assemble_turn_harness) takes this bundle instead of the raw provider, so
/// the harness assembly is expressed purely in crate model types; the
/// `Provider` → `ChatModel` adaptation is confined to `build_turn_models`.
pub(crate) struct TurnModels {
    /// The turn's effective/primary model (registry default + dispatch target).
    pub(crate) primary: TurnChatModel,
    /// Additive workload-tier routes (registry name → model), excluding the
    /// primary; the crate registry resolves fallback/selection across them.
    pub(crate) routes: TierRoutes,
    /// A model for the context-window summarizer (a distinct adapter instance so
    /// its provider errors don't touch the turn's `error_slot`).
    pub(crate) summarizer: TurnChatModel,
    /// Recovers the primary's original (downcastable) provider error on failure.
    pub(crate) error_slot: crate::agent::tinyagents::model::ModelErrorSlot,
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
}

impl TurnModels {
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

/// Host resolver for one live invocation. It exposes the exact pre-built
/// primary and fallback route models already selected by OpenHuman, rather
/// than constructing a fresh config-routed model during hosted preparation.
pub(crate) struct TurnModelResolver {
    primary: TurnChatModel,
    routes: std::collections::HashMap<String, TurnChatModel>,
}

impl TurnModelResolver {
    pub(crate) fn from_turn_models(models: &TurnModels) -> Self {
        Self {
            primary: models.primary.clone(),
            routes: models.routes.iter().cloned().collect(),
        }
    }
}

#[async_trait]
impl ModelResolver<()> for TurnModelResolver {
    async fn resolve(
        &self,
        request: &ModelResolveRequest,
    ) -> tinyagents_harness::Result<TurnChatModel> {
        Ok(request
            .model_pin()
            .and_then(|name| self.routes.get(name))
            .cloned()
            .unwrap_or_else(|| self.primary.clone()))
    }
}

/// Build the per-turn [`TurnModels`] **crate-natively** from `(role, config)` —
/// the Phase 3 P3-B cutover of [`build_turn_models`]: instead of wrapping one host
/// `Provider` per tier in a [`native model adapter`], each tier is built as a crate-native
/// [`ChatModel`] via [`factory::create_turn_chat_model`] (managed →
/// `OpenHumanBackendModel`, local/cloud → crate `OpenAiModel`).
///
/// The `TurnModels` shape is identical to [`build_turn_models`] so
/// [`assemble_turn_harness`](super::harness_assembly::assemble_turn_harness) is unchanged. The provider metadata
/// (`provider_id` / `native_tools` / `supports_vision`) is derived by the caller
/// ([`TurnModelSource::build`]) from the resolved provider string and config.
/// `error_slot` is a fresh
/// empty slot — crate-native models surface `TinyAgentsError` directly (no
/// downcastable `anyhow` to preserve), so typed provider-error *recovery* is unused
/// here (Sentry suppression is unaffected — both `skips_sentry` cases are raised in
/// the host turn loop).
#[allow(clippy::too_many_arguments)]
fn build_turn_models_crate(
    role: &str,
    config: &crate::config::Config,
    model: &str,
    temperature: f64,
    context_window: Option<u64>,
    primary_override: Option<&str>,
    provider_id: String,
    native_tools: bool,
    supports_vision: bool,
    force_text_mode: bool,
    thread_id: Option<&str>,
) -> anyhow::Result<TurnModels> {
    use crate::inference::provider::factory;

    // The primary honours an explicit provider-string override when the producer's
    // effective provider differs from `provider_for_role(role)` (triage #1257).
    let build_primary = |m: &str| -> anyhow::Result<TurnChatModel> {
        let managed = primary_override
            .map(|provider| {
                let provider = provider.trim();
                provider.is_empty() || provider == "cloud" || provider == "openhuman"
            })
            .unwrap_or_else(|| factory::resolves_to_managed_backend(role, config));
        if managed {
            let (backend, _) = factory::make_openhuman_backend_model_for_thread(
                role,
                config,
                m,
                !force_text_mode,
                thread_id,
            )?;
            return Ok(Arc::new(RouteRecordingModel::new(
                backend,
                ResolvedModelRoute::new("openhuman", m, m),
            )));
        }
        let (model, provider, resolved_model) = match primary_override {
            Some(ps) => factory::create_turn_chat_model_from_string_with_native_tools_and_route(
                role,
                ps,
                config,
                m,
                temperature,
                !force_text_mode,
            ),
            None => factory::create_turn_chat_model_with_native_tools_and_route(
                role,
                config,
                m,
                temperature,
                !force_text_mode,
            ),
        }?;
        Ok(Arc::new(RouteRecordingModel::new(
            model,
            ResolvedModelRoute::new(provider, resolved_model, m),
        )))
    };

    // Build the primary, every workload-tier route, and the summarizer under one
    // per-turn egress-dedup ledger: each managed construction resolves through the
    // same `resolve_managed_backend` chokepoint and would otherwise publish a
    // separate `ExternalTransferPending` for the same logical destination (codex
    // P2, PR #4812). `dedup_turn_scope` collapses same-destination repeats to one
    // disclosure per turn while still surfacing each distinct tier model.
    let (primary, routes, summarizer): BuiltTurnModels =
        crate::security::egress::dedup_turn_scope(|| {
            let primary = build_primary(model)?;

            // Additive workload-tier routes: one crate-native model per tier (skipping the
            // turn's own model, which is registered as the default primary), each pinned to
            // the tier alias so the crate registry resolves cross-route fallback across them.
            let mut routes: TierRoutes = Vec::new();
            for &tier in routes::WORKLOAD_ROUTE_TIERS {
                if tier == model {
                    continue;
                }
                let tier_role = factory::role_for_model_tier(tier);
                let route = if factory::resolves_to_managed_backend(tier_role, config) {
                    factory::make_openhuman_backend_model_for_thread(
                        tier_role,
                        config,
                        tier,
                        !force_text_mode,
                        thread_id,
                    )
                    .map(|(backend, _)| (backend, "openhuman".to_string(), tier.to_string()))
                } else {
                    factory::create_turn_chat_model_with_native_tools_and_route(
                        tier_role,
                        config,
                        tier,
                        temperature,
                        !force_text_mode,
                    )
                };
                match route {
                    Ok((route_model, provider, resolved_model)) => routes.push((
                        tier.to_string(),
                        Arc::new(RouteRecordingModel::new(
                            route_model,
                            ResolvedModelRoute::new(provider, resolved_model, tier),
                        )),
                    )),
                    Err(e) => {
                        // A route that can't be built (e.g. an unconfigured BYOK tier) is
                        // skipped, not fatal — the primary still dispatches (parity with the
                        // `Provider` path, where an unresolved tier simply isn't registered).
                        tracing::debug!(
                            route = tier,
                            error = %e,
                            "[models] skipping crate-native workload route that failed to build"
                        );
                    }
                }
            }

            // The summarizer is a distinct adapter instance (own empty error slot).
            let summarizer = build_primary(model)?;

            anyhow::Ok((primary, routes, summarizer))
        })?;

    Ok(TurnModels {
        primary,
        routes,
        summarizer,
        error_slot: Arc::new(std::sync::Mutex::new(None)),
        provider_id,
        context_window,
        native_tools,
        supports_vision,
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
    pub(crate) direct_model: Option<TurnChatModel>,
    /// When set, [`build`](Self::build) / [`build_summarizer`](Self::build_summarizer)
    /// construct **crate-native** models from `(role, config)` (Phase 3 P3-B) via
    /// [`build_turn_models_crate`]. Crate-native sources keep `provider` as
    /// `None`; build failures propagate instead of falling back to the host wire
    /// client.
    pub(crate) crate_native: Option<CrateNativeSource>,
    force_text_mode: bool,
}

/// The `(role, config)` a crate-native [`TurnModelSource`] builds its tiered
/// [`TurnModels`] from per turn.
#[derive(Clone)]
pub(crate) struct CrateNativeSource {
    role: String,
    config: Arc<crate::config::Config>,
    /// An explicit provider string for the **primary** model, overriding the
    /// role's default resolution. Set when a producer's effective provider differs
    /// from `provider_for_role(role)` — e.g. triage's #1257 force-managed override
    /// (`build_remote_provider`). `None` builds the primary from `role`. Routes
    /// always use the standard workload tiers.
    primary_override: Option<String>,
    pub(crate) force_text_mode: bool,
}

impl TurnModelSource {
    /// Use an already-constructed TinyAgents model as the complete turn model
    /// source. Intended for deterministic tests and embedding callers that do
    /// not need role/config-based route construction.
    pub fn from_model(model: TurnChatModel) -> Self {
        Self {
            direct_model: Some(model),
            crate_native: None,
            force_text_mode: false,
        }
    }

    /// Inject a model while supplying capability metadata that the model itself
    /// does not expose (common for deterministic scripted tests).
    pub(crate) fn from_model_with_profile(
        model: TurnChatModel,
        profile: tinyinference_llm::model::ModelProfile,
    ) -> Self {
        Self::from_model(Arc::new(ProfileOverrideModel::new(model, profile)))
    }

    /// Build a crate-native source: [`build`](Self::build) constructs the tiered
    /// [`TurnModels`] from `(role, config)` via [`build_turn_models_crate`] rather
    /// than wrapping a provider in `native model adapters. Used by the session-builder producer
    /// (`crate_native_provider`); the triage path uses
    /// [`new_crate_native_from_string`](Self::new_crate_native_from_string).
    pub(crate) fn new_crate_native(
        role: impl Into<String>,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            direct_model: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                primary_override: None,
                force_text_mode: false,
            }),
            force_text_mode: false,
        }
    }

    /// Build a crate-native source whose **primary** model is built from an explicit
    /// `provider_string` (via [`factory::create_turn_chat_model_from_string`]) rather
    /// than the role's default resolution — the triage path's #1257 force-managed
    /// override (`build_remote_provider` picks the effective string). Routes still
    /// use the standard workload tiers.
    pub(crate) fn new_crate_native_from_string(
        role: impl Into<String>,
        provider_string: impl Into<String>,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            direct_model: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                primary_override: Some(provider_string.into()),
                force_text_mode: false,
            }),
            force_text_mode: false,
        }
    }

    /// Force prompt-guided tool calling without resolving a host provider.
    pub(crate) fn with_text_mode(mut self) -> Self {
        self.force_text_mode = true;
        if let Some(source) = self.crate_native.as_mut() {
            source.force_text_mode = true;
        }
        self
    }

    /// Resolve the model's effective context window (async provider probe) — the
    /// value that drives the context-window summarization step. Resolved before
    /// [`build`](Self::build) so the harness graph makes no async `Provider` call.
    pub(crate) async fn effective_context_window(&self, model: &str) -> Option<u64> {
        if let Some(direct) = &self.direct_model {
            return direct
                .profile()
                .and_then(|profile| profile.max_input_tokens);
        }
        let provider_string = self.crate_native.as_ref().map(|source| {
            source.primary_override.clone().unwrap_or_else(|| {
                crate::inference::provider::provider_for_role(&source.role, &source.config)
            })
        });
        let local_kind = provider_string
            .as_deref()
            .and_then(tinyinference_local::profile::kind_from_provider_string);
        tinyinference_local::profile::context_window_with_local_fallback(
            model,
            crate::inference::model_context::context_window_for_model(model),
            local_kind,
        )
    }

    /// Whether the underlying provider is a local runtime (Ollama / LM Studio).
    /// A passthrough so callers (e.g. the sub-agent summarization-route decision)
    /// can branch on locality without naming the `Provider` trait.
    pub(crate) fn is_local_provider(&self) -> bool {
        if let Some(direct) = &self.direct_model {
            return direct
                .profile()
                .and_then(|profile| profile.provider.as_deref())
                .is_some_and(|provider| provider.eq_ignore_ascii_case("local"));
        }
        self.crate_native.as_ref().is_some_and(|source| {
            let provider = source.primary_override.clone().unwrap_or_else(|| {
                crate::inference::provider::provider_for_role(&source.role, &source.config)
            });
            tinyinference_local::profile::is_local_provider_string(&provider)
        })
    }

    /// Build this turn's [`TurnModels`] (primary + tier routes + summarizer),
    /// capturing provider telemetry id + capabilities onto the bundle.
    pub(crate) fn build(
        &self,
        model: &str,
        temperature: f64,
        context_window: Option<u64>,
        thread_id: Option<&str>,
    ) -> anyhow::Result<TurnModels> {
        if let Some(direct) = &self.direct_model {
            let mut profile = direct.profile().cloned().unwrap_or_default();
            if let Some(window) = context_window.filter(|window| *window > 0) {
                profile.max_input_tokens = Some(window);
            }
            if self.force_text_mode {
                profile.tool_calling = false;
                profile.parallel_tool_calls = false;
            }
            let provider_id = profile
                .provider
                .clone()
                .unwrap_or_else(|| "injected".to_string());
            let native_tools = profile.tool_calling;
            let supports_vision = profile.modalities.image_in;
            let context_window = context_window.or(profile.max_input_tokens);
            let primary: TurnChatModel = Arc::new(
                ProfileOverrideModel::new(direct.clone(), profile)
                    .with_request_model(model)
                    .with_request_temperature(temperature),
            );
            return Ok(TurnModels {
                primary,
                routes: Vec::new(),
                summarizer: direct.clone(),
                error_slot: Arc::new(std::sync::Mutex::new(None)),
                provider_id,
                context_window,
                native_tools,
                supports_vision,
            });
        }
        if let Some(cn) = &self.crate_native {
            let provider_string = cn.primary_override.clone().unwrap_or_else(|| {
                crate::inference::provider::provider_for_role(&cn.role, &cn.config)
            });
            let is_local = tinyinference_local::profile::is_local_provider_string(&provider_string);
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
            return build_turn_models_crate(
                &cn.role,
                &cn.config,
                model,
                temperature,
                context_window,
                cn.primary_override.as_deref(),
                provider_id,
                !is_local,
                !is_local,
                cn.force_text_mode,
                thread_id,
            );
        }
        Err(anyhow::anyhow!("turn model source is missing a model"))
    }

    /// Build a standalone summarizer [`ChatModel`](tinyinference_llm::model::ChatModel)
    /// over this source's provider — a fresh adapter (own error slot) for one-off
    /// summary calls outside the main turn (e.g. the sub-agent cap-hit checkpoint),
    /// so the caller can `invoke` without naming the `Provider` trait. The output
    /// cap rides the per-call `ModelRequest`, not the model.
    pub(crate) fn build_summarizer(
        &self,
        model: &str,
        temperature: f64,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Arc<dyn tinyinference_llm::model::ChatModel<()>>> {
        if let Some(direct) = &self.direct_model {
            let profile = direct.profile().cloned().unwrap_or_default();
            return Ok(Arc::new(
                ProfileOverrideModel::new(direct.clone(), profile)
                    .with_request_model(model)
                    .with_request_temperature(temperature),
            ));
        }
        if let Some(cn) = &self.crate_native {
            let managed = cn
                .primary_override
                .as_deref()
                .map(|provider| {
                    let provider = provider.trim();
                    provider.is_empty() || provider == "cloud" || provider == "openhuman"
                })
                .unwrap_or_else(|| {
                    crate::inference::provider::factory::resolves_to_managed_backend(
                        &cn.role, &cn.config,
                    )
                });
            if managed {
                return crate::inference::provider::factory::make_openhuman_backend_model_for_thread(
                    &cn.role,
                    &cn.config,
                    model,
                    !cn.force_text_mode,
                    thread_id,
                )
                .map(|(model, _)| model);
            }
            let built = match cn.primary_override.as_deref() {
                Some(ps) => {
                    crate::inference::provider::factory::create_turn_chat_model_from_string(
                        &cn.role,
                        ps,
                        &cn.config,
                        model,
                        temperature,
                    )
                }
                None => crate::inference::provider::factory::create_turn_chat_model(
                    &cn.role,
                    &cn.config,
                    model,
                    temperature,
                ),
            };
            return built;
        }
        Err(anyhow::anyhow!("turn model source is missing a model"))
    }
}
