//! Workload-route → model-registry projection (issue #4249, Workstream 02.1).
//!
//! `provider/router.rs` owns the product policy that maps a workload **tier
//! name** (`chat`, `reasoning`, `agentic`, `coding`, `burst`, `summarization`,
//! `vision`) to a concrete provider + model. This module is a thin *projection*
//! of that route set into `tinyagents` [`ProviderModel`] registry entries: for
//! each route it builds a [`ProviderModel`] carrying a real [`ModelProfile`]
//! (per-route vision/reasoning capability + context window) so the crate's
//! registry can resolve and capability-check the full route set — the enabler
//! for SDK-owned fallback (02.2) and the model catalog (02.4).
//!
//! It does **not** move route policy into the crate: the dispatch model string
//! for each entry is the OpenHuman tier alias (`chat-v1`, `reasoning-v1`, …),
//! which the wrapped [`Provider`] (a `RouterProvider` for BYOK, or the managed
//! backend) resolves to a concrete model at call time exactly as it does today.
//! Registering the extra routes is additive: `set_default_model` still points at
//! the turn's effective model, so nothing dispatches to these entries until a
//! future fallback/selection step chooses them.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::harness::context::RunContext;
use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::middleware::{MiddlewareModelOutcome, ModelHandler, ModelMiddleware};
use tinyagents::harness::model::{CapabilitySet, ModelRequest};
use tinyagents::harness::retry::FallbackPolicy;

use crate::openhuman::config::{
    MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_V1,
    MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
};
use crate::openhuman::inference::model_context::context_window_for_model;
use crate::openhuman::inference::provider::factory::oh_tier_supports_vision;
use crate::openhuman::inference::provider::Provider;

use super::model::ProviderModel;

/// The workload routes projected into the registry, keyed by their OpenHuman
/// tier alias (the string the wrapped provider resolves at dispatch).
///
/// This mirrors the tier-name set `provider/router.rs::openhuman_tier_to_hint`
/// recognizes (`reasoning`, `chat`, `agentic`, `burst`, `coding`,
/// `summarization`, `vision`). `router.rs` stays the product source of truth for
/// which provider/model each name resolves to; this list is only the projection
/// inventory. `subconscious`/`memory` are intentionally absent — they are role
/// aliases that ride the `chat-v1` model rather than distinct router tiers.
pub(super) const WORKLOAD_ROUTE_TIERS: &[&str] = &[
    MODEL_CHAT_V1,
    MODEL_REASONING_V1,
    MODEL_AGENTIC_V1,
    MODEL_CODING_V1,
    MODEL_BURST_V1,
    MODEL_SUMMARIZATION_V1,
    MODEL_VISION_V1,
];

/// Whether a workload tier emits reasoning/thinking output.
///
/// Static, tier-identity based: only the dedicated reasoning tier is projected
/// as reasoning-capable. There is no per-tier reasoning accessor on the managed
/// backend yet (mirrors the vision map in `factory::oh_tier_supports_vision`);
/// flip an arm here once one exists.
fn tier_supports_reasoning(tier: &str) -> bool {
    tier == MODEL_REASONING_V1
}

/// One projected registry entry: the registry name (dispatch model alias) and
/// its capability-carrying [`ProviderModel`] adapter.
pub(super) struct RouteModel {
    pub(super) name: String,
    pub(super) model: Arc<ProviderModel>,
}

/// Build the [`ProviderModel`] registry entries for every resolvable workload
/// route, excluding `skip_model` (the turn's effective/primary model, which the
/// caller registers separately and keeps as the default).
///
/// Each entry wraps the same `provider` handle under a tier-alias model string
/// and records the route's real [`ModelProfile`]: per-route vision
/// (`factory::oh_tier_supports_vision`), reasoning ([`tier_supports_reasoning`]),
/// and context window (`model_context::context_window_for_model`). Tool-calling
/// and streaming flags come from the wrapped provider (as
/// [`ProviderModel::new`] derives them). A route whose context window cannot be
/// resolved is still registered (window is optional metadata) but logged; the
/// projection never fails a turn.
pub(super) fn build_route_models(
    provider: &Arc<dyn Provider>,
    temperature: f64,
    skip_model: &str,
    max_output_tokens: Option<u32>,
) -> Vec<RouteModel> {
    let mut out = Vec::new();
    for &tier in WORKLOAD_ROUTE_TIERS {
        if tier == skip_model {
            // The turn's own model is registered (and set as default) by the
            // caller; don't shadow it.
            continue;
        }
        let vision = oh_tier_supports_vision(tier);
        let reasoning = tier_supports_reasoning(tier);
        let window = context_window_for_model(tier);
        if window.is_none() {
            tracing::debug!(
                route = tier,
                "[models] projecting workload route with no known context window"
            );
        }
        let mut model = ProviderModel::new(provider.clone(), tier, temperature)
            .with_vision(vision)
            .with_reasoning(reasoning);
        if let Some(cap) = max_output_tokens {
            model = model.with_max_tokens(cap);
        }
        if let Some(window) = window.filter(|w| *w > 0) {
            model = model.with_context_window(window);
        }
        tracing::debug!(
            route = tier,
            vision,
            reasoning,
            context_window = window,
            "[models] registered workload route as registry entry"
        );
        out.push(RouteModel {
            name: tier.to_string(),
            model: Arc::new(model),
        });
    }
    out
}

/// The capability needs a turn imposes on every model call, derived from what is
/// cheaply available at harness-assembly time.
///
/// Today the only reliably-derivable, safe-to-require signal is **vision**: when
/// the turn's effective model is the dedicated `vision-v1` tier the turn was
/// routed there because it carries image input (this is exactly what the
/// `model_vision` selection in `subagent_runner/ops/graph.rs` encodes), so we
/// require `image_in` — which keeps the primary vision model selectable while
/// filtering any non-vision fallback pre-dispatch.
///
/// Returns `None` (install no gate) when no requirement is derivable, so the
/// common text turn is unaffected. Signals still to thread (see module note and
/// the migration spec): per-call tool-calling and reasoning needs, BYOK vision
/// (needs `Config` + `model_registry.vision`), and true per-message image
/// presence rather than the tier proxy.
pub(super) fn turn_required_capabilities(model: &str) -> Option<CapabilitySet> {
    if model == MODEL_VISION_V1 || model == "hint:vision" {
        return Some(CapabilitySet {
            image_in: true,
            ..CapabilitySet::default()
        });
    }
    None
}

/// Around-model middleware that stamps the turn's required [`CapabilitySet`] onto
/// every [`ModelRequest`] before resolution/dispatch, so the crate rejects an
/// unfit model pre-dispatch (and, once fallback is wired in 02.2, selects the
/// next capable route) instead of failing at the provider.
///
/// It only sets the requirement when the request carries none, so an inner layer
/// that already declared stricter needs wins.
pub(super) struct RequiredCapabilitiesMiddleware {
    required: CapabilitySet,
}

impl RequiredCapabilitiesMiddleware {
    pub(super) fn new(required: CapabilitySet) -> Self {
        Self { required }
    }
}

#[async_trait]
impl ModelMiddleware<()> for RequiredCapabilitiesMiddleware {
    fn name(&self) -> &str {
        "openhuman.required_capabilities"
    }

    async fn wrap_model(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        mut request: ModelRequest,
        next: ModelHandler<'_, (), ()>,
    ) -> tinyagents::Result<MiddlewareModelOutcome> {
        if request.required_capabilities.is_none() {
            request = request.with_required_capabilities(self.required.clone());
        }
        next.run(ctx, state, request).await
    }
}

/// The ordered same-family fallback **alternates** for a workload tier (issue
/// #4249, Workstream 02.2). Each primary tier falls back to a single sibling in
/// the same workload family so the harness can retry a different registered route
/// when the primary route errors — bounding the cross-route fan-out to one extra
/// call per turn (mirroring the single-alternate spirit of `ReliableProvider`'s
/// `model_fallbacks`, so this does not regress failure latency/cost).
///
/// Ordering rationale (derived from `provider/router.rs` tier families):
/// - `chat-v1` ⇄ `burst-v1` — the two light/fast conversational tiers.
/// - `reasoning-v1` ⇄ `agentic-v1` — the two heavy reasoning/agentic tiers.
/// - `coding-v1` → `agentic-v1` — coding is agentic-adjacent (tool-heavy).
/// - `summarization-v1` → `chat-v1` — summarization rides a general chat model.
/// - `vision-v1` → **none**: a non-vision route cannot satisfy the `image_in`
///   capability the [`RequiredCapabilitiesMiddleware`] stamps on a vision turn, so
///   falling back to a text tier would only be rejected pre-dispatch. There is a
///   single vision tier, so vision turns are primary-only.
///
/// A model that is not one of the recognized tiers (a raw provider model string)
/// returns an empty slice → no fallback chain is installed for that turn.
fn same_family_fallbacks(model: &str) -> &'static [&'static str] {
    match model {
        MODEL_CHAT_V1 => &[MODEL_BURST_V1],
        MODEL_BURST_V1 => &[MODEL_CHAT_V1],
        MODEL_REASONING_V1 => &[MODEL_AGENTIC_V1],
        MODEL_AGENTIC_V1 => &[MODEL_REASONING_V1],
        MODEL_CODING_V1 => &[MODEL_AGENTIC_V1],
        MODEL_SUMMARIZATION_V1 => &[MODEL_CHAT_V1],
        MODEL_VISION_V1 => &[],
        _ => &[],
    }
}

/// Build the [`FallbackPolicy`] for a turn whose effective/primary model is
/// `model` (issue #4249, Workstream 02.2). The returned chain is `[primary,
/// alternate…]` — the crate's [`FallbackPolicy::next_after`] traversal expects the
/// current (primary) name as the first entry and yields each subsequent alternate.
///
/// Every alternate is a distinct workload tier that
/// [`build_route_models`] has already registered in the harness model registry
/// (the primary tier itself is skipped there, since the caller registers it as the
/// default), so the harness can resolve each fallback name to its capability-carrying
/// route adapter. Returns `None` when no same-family alternate exists (vision, or a
/// raw non-tier model string), leaving the turn primary-only.
pub(super) fn route_fallback_policy(model: &str) -> Option<FallbackPolicy> {
    let alternates = same_family_fallbacks(model);
    if alternates.is_empty() {
        tracing::debug!(
            route = model,
            "[fallback] no same-family fallback route; turn is primary-only"
        );
        return None;
    }
    let mut models = Vec::with_capacity(alternates.len() + 1);
    models.push(model.to_string());
    models.extend(alternates.iter().map(|s| s.to_string()));
    tracing::debug!(
        route = model,
        chain = ?models,
        "[fallback] configured SDK-owned cross-route fallback chain"
    );
    Some(FallbackPolicy { models })
}

/// Around-model middleware that makes the crate's registry-backed
/// [`RunPolicy::fallback`][tinyagents::harness::runtime::RunPolicy] traversal
/// **event-visible** (issue #4249, Workstream 02.2).
///
/// The harness performs the cross-route fallback swap inside its model-resolving
/// core (`agent_loop::invoke_model_resolving`) but — unlike the
/// [`ModelFallbackMiddleware`][tinyagents::harness::middleware::ModelFallbackMiddleware]
/// primitive — that native path emits **no**
/// [`AgentEvent::FallbackSelected`]. This observer wraps the resolving core, and
/// on success compares the response's `resolved_model` against the turn's primary
/// model name: when they differ a fallback occurred, so it emits the parity
/// `FallbackSelected` event (mirrored onto OpenHuman's progress/observability
/// bridge) and logs it under `[fallback]`. It never re-issues the call, so it adds
/// no extra provider dispatch on top of the native traversal (no double-fallback).
pub(super) struct FallbackObserverMiddleware {
    primary: String,
}

impl FallbackObserverMiddleware {
    pub(super) fn new(primary: impl Into<String>) -> Self {
        Self {
            primary: primary.into(),
        }
    }
}

#[async_trait]
impl ModelMiddleware<()> for FallbackObserverMiddleware {
    fn name(&self) -> &str {
        "openhuman.fallback_observer"
    }

    async fn wrap_model(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        request: ModelRequest,
        next: ModelHandler<'_, (), ()>,
    ) -> tinyagents::Result<MiddlewareModelOutcome> {
        let outcome = next.run(ctx, state, request).await?;
        let response = outcome.into_response();
        if let Some(resolved) = response.resolved_model.as_ref() {
            if resolved.name != self.primary {
                tracing::info!(
                    from = %self.primary,
                    to = %resolved.name,
                    "[fallback] SDK selected a cross-route fallback model after the primary route failed"
                );
                ctx.emit(AgentEvent::FallbackSelected {
                    from: self.primary.clone(),
                    to: resolved.name.clone(),
                });
            }
        }
        Ok(MiddlewareModelOutcome::from(response))
    }
}
