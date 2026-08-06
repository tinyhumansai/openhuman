//! Workload routing and model-call middleware for native TinyAgents models.

use std::sync::LazyLock;

use async_trait::async_trait;
use tinyagents::harness::context::RunContext;
use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::middleware::{MiddlewareModelOutcome, ModelHandler, ModelMiddleware};
use tinyagents::harness::model::{CapabilitySet, ModelRequest};
use tinyagents::harness::retry::FallbackPolicy;
use tinyagents::registry::{ModelRouter, WorkloadRoute};

use crate::openhuman::config::{
    MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_V1,
    MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
};

/// The workload routes projected into the registry, keyed by their OpenHuman
/// tier alias (the string the wrapped provider resolves at dispatch).
///
/// This is the canonical tier inventory (`reasoning`, `chat`, `agentic`,
/// `burst`, `coding`, `summarization`, `vision`). The inference provider factory
/// resolves the selected tier to its configured model. `subconscious`/`memory`
/// are intentionally absent — they are role aliases that ride the `chat-v1`
/// model rather than distinct router tiers.
pub(super) const WORKLOAD_ROUTE_TIERS: &[&str] = &[
    MODEL_CHAT_V1,
    MODEL_REASONING_V1,
    MODEL_AGENTIC_V1,
    MODEL_CODING_V1,
    MODEL_BURST_V1,
    MODEL_SUMMARIZATION_V1,
    MODEL_VISION_V1,
];

/// The OpenHuman workload-tier routing table as a crate
/// [`ModelRouter`](tinyagents::registry::ModelRouter) — the single declarative
/// source for cross-route **fallback chains** and per-tier **required-capability
/// gates** (issue #4249, Phase 3 routing consolidation).
///
/// The router owns the policy this module previously open-coded as
/// `same_family_fallbacks` +
/// `turn_required_capabilities`: it answers [`route_fallback_policy`] and
/// [`turn_required_capabilities`] from one declarative table.
///
/// Built once — the tier set + fallback ordering + vision gate are static:
/// - light/fast conversational siblings `chat-v1 ⇄ burst-v1`;
/// - heavy reasoning/agentic siblings `reasoning-v1 ⇄ agentic-v1`;
/// - `coding-v1 → agentic-v1` (coding is tool-heavy, agentic-adjacent);
/// - `summarization-v1 → chat-v1` (summarization rides a general chat model);
/// - `vision-v1` is `image_in`-gated and primary-only — a text fallback cannot
///   satisfy the gate — and its `hint:vision` form carries the same gate.
static OH_WORKLOAD_ROUTER: LazyLock<ModelRouter> = LazyLock::new(|| {
    let vision_gate = CapabilitySet {
        image_in: true,
        ..CapabilitySet::default()
    };
    ModelRouter::new()
        .with_route(
            WorkloadRoute::new(MODEL_CHAT_V1, MODEL_CHAT_V1).with_fallbacks([MODEL_BURST_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_BURST_V1, MODEL_BURST_V1).with_fallbacks([MODEL_CHAT_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_REASONING_V1, MODEL_REASONING_V1)
                .with_fallbacks([MODEL_AGENTIC_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_AGENTIC_V1, MODEL_AGENTIC_V1)
                .with_fallbacks([MODEL_REASONING_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_CODING_V1, MODEL_CODING_V1).with_fallbacks([MODEL_AGENTIC_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_SUMMARIZATION_V1, MODEL_SUMMARIZATION_V1)
                .with_fallbacks([MODEL_CHAT_V1]),
        )
        .with_route(
            WorkloadRoute::new(MODEL_VISION_V1, MODEL_VISION_V1).requiring(vision_gate.clone()),
        )
        // The hint form resolves to the same vision tier and carries the same gate,
        // with no fallback (primary-only), matching the legacy static gate.
        .with_route(WorkloadRoute::new("hint:vision", MODEL_VISION_V1).requiring(vision_gate))
});

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
    OH_WORKLOAD_ROUTER.required_capabilities(model)
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

/// Build the [`FallbackPolicy`] for a turn whose effective/primary model is
/// `model` (issue #4249, Workstream 02.2). The returned chain is `[primary,
/// alternate…]` — the crate's [`FallbackPolicy::next_after`] traversal expects the
/// current (primary) name as the first entry and yields each subsequent alternate.
///
/// The chain now comes straight from the declarative [`OH_WORKLOAD_ROUTER`]
/// (`fallback_policy` leads with the primary, then the tier's same-family
/// alternates). Returns `None` when no same-family alternate exists (vision, or
/// a raw non-tier model string), leaving the turn primary-only.
pub(super) fn route_fallback_policy(model: &str) -> Option<FallbackPolicy> {
    let policy = OH_WORKLOAD_ROUTER.fallback_policy(model);
    match &policy {
        Some(p) => tracing::debug!(
            route = model,
            chain = ?p.models,
            "[fallback] configured SDK-owned cross-route fallback chain"
        ),
        None => tracing::debug!(
            route = model,
            "[fallback] no same-family fallback route; turn is primary-only"
        ),
    }
    policy
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

/// Around-model middleware that feeds the cost event bridge (issue #4249,
/// Phase 5): after the real model call, it reads the full host [`UsageInfo`] off
/// the returned [`ModelResponse`] — token breakdowns from the crate `Usage`,
/// backend-charged USD + context window from the G1 `raw` passthrough
/// ([`usage_info_from_response`](super::model::usage_info_from_response)) — and
/// pushes it onto the shared [`ProviderUsageCarry`](super::observability::ProviderUsageCarry)
/// the [`OpenhumanEventBridge`](super::OpenhumanEventBridge) drains on
/// `UsageRecorded`.
///
/// It wraps the whole retry/fallback core, so it fires
/// exactly once per logical model call (matching the single `UsageRecorded` the
/// crate emits), for both the buffered and streamed paths (the streamed response
/// is folded back to a `ModelResponse` with usage + raw intact). Push happens
/// after the call returns, before the loop emits `UsageRecorded`, preserving the
/// FIFO ordering the bridge relies on.
pub(super) struct UsageCarryMiddleware {
    carry: super::observability::ProviderUsageCarry,
}

impl UsageCarryMiddleware {
    pub(super) fn new(carry: super::observability::ProviderUsageCarry) -> Self {
        Self { carry }
    }
}

#[async_trait]
impl ModelMiddleware<()> for UsageCarryMiddleware {
    fn name(&self) -> &str {
        "openhuman.usage_carry"
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
        if let Some(usage) = super::model::usage_info_from_response(&response) {
            self.carry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push_back(usage);
        }
        Ok(MiddlewareModelOutcome::from(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback chain for every tier must lead with the primary and carry the
    /// single same-family alternate the legacy static table encoded — the crate
    /// `ModelRouter` projection is exactly behavior-neutral.
    #[test]
    fn route_fallback_policy_matches_legacy_chains() {
        let cases: &[(&str, Option<&[&str]>)] = &[
            (MODEL_CHAT_V1, Some(&[MODEL_CHAT_V1, MODEL_BURST_V1])),
            (MODEL_BURST_V1, Some(&[MODEL_BURST_V1, MODEL_CHAT_V1])),
            (
                MODEL_REASONING_V1,
                Some(&[MODEL_REASONING_V1, MODEL_AGENTIC_V1]),
            ),
            (
                MODEL_AGENTIC_V1,
                Some(&[MODEL_AGENTIC_V1, MODEL_REASONING_V1]),
            ),
            (MODEL_CODING_V1, Some(&[MODEL_CODING_V1, MODEL_AGENTIC_V1])),
            (
                MODEL_SUMMARIZATION_V1,
                Some(&[MODEL_SUMMARIZATION_V1, MODEL_CHAT_V1]),
            ),
            // Vision is primary-only (an image_in gate no text tier can satisfy).
            (MODEL_VISION_V1, None),
            ("hint:vision", None),
            // A raw non-tier model installs no chain.
            ("gpt-4o", None),
        ];
        for (model, expected) in cases {
            let got = route_fallback_policy(model).map(|p| p.models);
            let want =
                expected.map(|chain| chain.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            assert_eq!(got, want, "fallback chain mismatch for {model}");
        }
    }

    /// Only the vision tier (and its hint form) imposes an `image_in` gate; the
    /// common text turn stays ungated.
    #[test]
    fn turn_required_capabilities_gates_only_vision() {
        let vision = turn_required_capabilities(MODEL_VISION_V1).expect("vision is gated");
        assert!(vision.image_in);
        let hint = turn_required_capabilities("hint:vision").expect("hint:vision is gated");
        assert!(hint.image_in);
        for model in [
            MODEL_CHAT_V1,
            MODEL_REASONING_V1,
            MODEL_AGENTIC_V1,
            MODEL_CODING_V1,
            MODEL_BURST_V1,
            MODEL_SUMMARIZATION_V1,
            "gpt-4o",
        ] {
            assert!(
                turn_required_capabilities(model).is_none(),
                "{model} must not be capability-gated"
            );
        }
    }

    /// The router covers exactly the projected tier inventory (plus the hint:vision
    /// gate alias), so the fallback/capability source of truth stays aligned with
    /// `WORKLOAD_ROUTE_TIERS`.
    #[test]
    fn router_covers_the_workload_tier_inventory() {
        for tier in WORKLOAD_ROUTE_TIERS {
            assert!(
                OH_WORKLOAD_ROUTER.route(tier).is_some(),
                "router missing tier {tier}"
            );
        }
    }
}
