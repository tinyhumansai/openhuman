//! Host capability: **which** model answers a turn.
//!
//! Adapts [`tinyagents_harness::host::ModelResolver`] onto OpenHuman's
//! inference domain — `crate::openhuman::inference::provider` (the per-role
//! provider factory: [`create_chat_model_with_model_id`], `provider_for_role`,
//! [`role_for_model_tier`]) driven by a [`Config`] snapshot.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The crate deliberately refuses
//! to know about tiers, lead/subagent economics, or BYOK-vs-managed routing —
//! all of that is OpenHuman product policy, and this module is where it lives.
//! The mapping performed here is exactly the one the rest of the core already
//! performs: *workload role* → provider string → concrete `ChatModel`. Nothing
//! about provider selection, BYOK inheritance, or egress disclosure is
//! re-implemented; it is all reached through `create_chat_model_with_model_id`,
//! which is the single chokepoint that emits the `ExternalTransfer` egress
//! disclosure. Building a client by any other route would bypass that guard.
//!
//! # Contract mismatches resolved here
//!
//! 1. **`State` erasure.** The trait is generic over the harness state
//!    (`ModelResolver<State>` must return `Arc<dyn ChatModel<State>>`), but
//!    every model OpenHuman builds is a `ChatModel<()>` — the core's harness
//!    carries its per-turn context in task-locals and `RunContext`, not in a
//!    typed state value. [`StatelessModel`] bridges the two: it implements
//!    `ChatModel<State>` for *any* `State` by discarding the state reference and
//!    invoking the inner model with `&()`. That is lossless today precisely
//!    because no OpenHuman model reads state; if one ever does, it must stop
//!    going through this wrapper rather than silently observing `()`.
//!
//! 2. **"resolve is cheap, and must not be memoized by the caller."** The trait
//!    tells the *runtime* not to cache, which leaves the host free to. A fresh
//!    `create_chat_model_with_model_id` per turn would hand back a new provider
//!    client and throw away its connection pool — the exact waste the trait's
//!    `Arc` return is trying to avoid. Since this resolver holds an immutable
//!    `Arc<Config>` snapshot, the answer for a given workload role cannot change
//!    over its lifetime, so [`OpenHumanModelResolver`] memoizes per role. A
//!    caller that needs re-resolution after a config edit constructs a new
//!    resolver, which is how every other `Arc<Config>` holder in the core works.
//!
//! 3. **Tiered routing is not represented.** OpenHuman's richer per-turn model
//!    bundle (primary + workload-tier fallback routes + summarizer) is
//!    `TurnModelSource` / `TurnModels` in `super::super` — a *bundle*, not one
//!    `Arc<dyn ChatModel>`, so it does not fit this signature. See the
//!    `TODO(phase4)` on [`OpenHumanModelResolver::base_model_for_role`].
//!
//! 4. **`ModelResolveRequest::model_pin` is not honoured yet.** The field
//!    exists now (`tinyagents#89`, which this resolver's `-v1` suffix test was
//!    written against while the pin had nowhere to go), so a definition's exact
//!    model id finally has a route here rather than being smuggled through
//!    `role`. It is deliberately still unread, for two reasons:
//!
//!    * **Nothing constructs a `ModelResolveRequest` yet.** The crate does not
//!      emit one and this resolver is not repointed, so honouring the pin today
//!      would be code with no producer and no test that could fail honestly.
//!    * **There is no build-by-exact-id seam to honour it with.**
//!      `create_chat_model_with_model_id` takes a *role*, not a model id, so
//!      honouring a pin means adding that path in
//!      `inference::provider::factory` and deciding what happens when the
//!      pinned id is not configured — a real design question, not plumbing.
//!
//!    When it is wired: the pin is **advisory**. The host decides whether it
//!    can be honoured, because the runtime has no view of credentials or
//!    provider health. Validate the id against configured providers and fall
//!    back to role routing with a warning; never pass it through blind.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents_harness::error::TinyAgentsError;
use tinyagents_harness::host::{ModelResolveRequest, ModelResolver};
use tinyagents_harness::Result as TaResult;
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream};

use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::{create_chat_model_with_model_id, role_for_model_tier};

/// The workload roles `provider_for_role` understands **and** that name a
/// chat-shaped model.
///
/// `embeddings` is deliberately absent even though the factory routes it: it
/// names an embedding model, and handing one back as the turn's `ChatModel`
/// would fail at dispatch rather than at resolution. An unrecognised role falls
/// back to the structural default instead (see [`workload_role_for`]).
const CHAT_WORKLOAD_ROLES: &[&str] = &[
    "chat",
    "reasoning",
    "agentic",
    "coding",
    "burst",
    "vision",
    "memory",
    "summarization",
    "heartbeat",
    "learning",
    "subconscious",
];

/// The role a team lead takes when the caller supplied none.
///
/// `chat`, matching the live session path rather than the tempting `agentic`.
///
/// A lead does drive the tool-heavy orchestration turn, so `agentic` looks
/// right — but `session/builder/factory.rs::provider_role_for` deliberately
/// sends the orchestrator and every non-`hint:*` model to `chat`, and pins it
/// with `orchestrator_defaults_to_chat`. That is what makes the user's
/// Connections → API keys → LLM **chat** provider drive the user-facing turn.
/// Defaulting a lead to `agentic` here would silently reroute it onto the
/// separately-configured agentic provider the moment this seam went live — a
/// behaviour change smuggled in as a default. `agentic` is reachable, but only
/// through an explicit `hint:agentic`.
const LEAD_DEFAULT_ROLE: &str = "chat";

/// The role a non-lead agent takes when the caller supplied none.
///
/// `chat` is the core's own default workload (`DEFAULT_MODEL` is `chat-v1`), so
/// an unannotated delegate lands exactly where an unconfigured OpenHuman turn
/// already lands.
const SUBAGENT_DEFAULT_ROLE: &str = "chat";

/// Whether `lowered` is a model-tier spelling `role_for_model_tier` recognises,
/// rather than merely something that ends in `-v1`.
///
/// Checks the stem against [`CHAT_WORKLOAD_ROLES`] instead of restating the
/// factory's tier table, so the two cannot drift. `reasoning-quick-v1` is the
/// one tier whose stem is not itself a workload role (it rides the chat model),
/// so it is named explicitly.
fn is_known_model_tier(lowered: &str) -> bool {
    let Some(stem) = lowered.strip_suffix("-v1") else {
        return false;
    };
    stem == "reasoning-quick" || CHAT_WORKLOAD_ROLES.contains(&stem)
}

/// Maps one [`ModelResolveRequest`] onto an OpenHuman workload role.
///
/// This function *is* the product policy the seam exists to hold:
///
/// * an explicit role wins, after normalisation — a caller may spell it as a
///   plain workload role (`"reasoning"`), as a model tier (`"reasoning-v1"`), or
///   as an agent-definition hint (`"hint:agentic"`), and all three are in live
///   use across `agent.toml` files and the channel routes;
/// * otherwise the structural lead/subagent split decides, which is the one
///   routing fact the runtime can assert honestly.
///
/// An unrecognised role is **not** an error: roles reach us as free-form data
/// that already passed config validation, and refusing to route would turn a
/// cosmetic typo into an agent that cannot run at all. It falls back to the
/// structural default with a warning — the same posture
/// `super::super::config::dispatcher_from` takes.
fn workload_role_for(req: &ModelResolveRequest) -> &'static str {
    let structural_default = if req.is_team_lead {
        LEAD_DEFAULT_ROLE
    } else {
        SUBAGENT_DEFAULT_ROLE
    };

    let Some(raw) = req.role() else {
        tracing::debug!(
            target: "tinyagents",
            agent_id = %req.agent_id,
            is_team_lead = req.is_team_lead,
            role = structural_default,
            "[tinyagents][model_resolver] no role supplied; using the structural default"
        );
        return structural_default;
    };

    let lowered = raw.to_ascii_lowercase();

    // Tier and hint spellings normalise through the factory's own table so this
    // adapter never re-derives the tier→role mapping (and cannot drift from it).
    //
    // `hint:` is an unambiguous prefix, so it routes unconditionally. `-v1` is
    // not: it is a *suffix heuristic*, and an exact model id can end in `-v1`
    // too. `role_for_model_tier` answers `"chat"` for anything it does not
    // recognise, and it cannot distinguish "this is the chat tier" from "I have
    // never heard of this" — so passing an arbitrary `-v1` id through it would
    // silently reroute to chat with no diagnostic at all. Only hand it values
    // whose stem is a workload role it will actually recognise; anything else
    // falls through to the unknown-role path below, which at least warns.
    if lowered.starts_with("hint:") || is_known_model_tier(&lowered) {
        return role_for_model_tier(&lowered);
    }

    if let Some(known) = CHAT_WORKLOAD_ROLES
        .iter()
        .find(|candidate| **candidate == lowered)
    {
        return known;
    }

    tracing::warn!(
        target: "tinyagents",
        agent_id = %req.agent_id,
        role = %raw,
        fallback = structural_default,
        "[tinyagents][model_resolver] unknown workload role; falling back to the structural default"
    );
    structural_default
}

/// Presents a state-agnostic OpenHuman `ChatModel<()>` as a `ChatModel<State>`
/// for any harness state.
///
/// Not a general-purpose adapter: it is sound only because OpenHuman's models
/// genuinely ignore the harness state (they carry per-turn context in
/// task-locals and `RunContext`). Both `invoke` and `stream` are forwarded so a
/// streaming provider keeps streaming — falling through to the trait's default
/// `stream` would silently downgrade every resolved model to replayed unary.
struct StatelessModel {
    inner: Arc<dyn ChatModel<()>>,
}

#[async_trait]
impl<State: Send + Sync> ChatModel<State> for StatelessModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    async fn invoke(
        &self,
        _state: &State,
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.inner.invoke(&(), request).await
    }

    async fn stream(
        &self,
        _state: &State,
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        self.inner.stream(&(), request).await
    }
}

/// OpenHuman's [`ModelResolver`]: routes a turn to a workload role, then to that
/// role's configured provider.
///
/// Holds an immutable [`Config`] snapshot — the same shape `TurnModelSource`'s
/// crate-native source uses — plus a per-role cache of already-built clients.
pub struct OpenHumanModelResolver {
    config: Arc<Config>,
    /// Temperature applied as the *request default* by the factory; an explicit
    /// per-call `ModelRequest` temperature still wins.
    temperature: f64,
    /// Built clients keyed by workload role. Guarded by a `std::sync::Mutex`
    /// held across no `.await` — model construction is synchronous.
    cache: Mutex<HashMap<&'static str, Arc<dyn ChatModel<()>>>>,
}

impl OpenHumanModelResolver {
    /// Builds a resolver over `config`, using its configured default
    /// temperature.
    pub fn new(config: Arc<Config>) -> Self {
        let temperature = config.default_temperature;
        Self {
            config,
            temperature,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Builds a resolver that pins every resolved model to `temperature`
    /// instead of the config default.
    pub fn with_temperature(config: Arc<Config>, temperature: f64) -> Self {
        Self {
            config,
            temperature,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The config snapshot this resolver routes against.
    pub fn config(&self) -> &Arc<Config> {
        &self.config
    }

    /// Resolves (and memoizes) the state-agnostic client for one workload role.
    ///
    /// TODO(phase4): this returns the role's **primary** model only. OpenHuman's
    /// real per-turn bundle — primary + workload-tier fallback routes +
    /// summarizer — is `TurnModels`, built by
    /// `crate::openhuman::agent::tinyagents::TurnModelSource::build` (see
    /// `build_turn_models_crate` in `src/openhuman/tinyagents/mod.rs`). That is a
    /// bundle rather than a single `Arc<dyn ChatModel>`, so it does not fit the
    /// `ModelResolver` signature; wiring cross-tier fallback behind this seam
    /// needs either a crate-side registry hand-off or a second host capability,
    /// and is out of scope for a Phase 4 adapter.
    fn base_model_for_role(&self, role: &'static str) -> TaResult<Arc<dyn ChatModel<()>>> {
        if let Some(hit) = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(role)
        {
            tracing::trace!(
                target: "tinyagents",
                role,
                "[tinyagents][model_resolver] reusing the cached client for this role"
            );
            return Ok(Arc::clone(hit));
        }

        // The one construction chokepoint: provider resolution, BYOK/local/managed
        // selection, and the egress disclosure all happen inside here.
        let (model, model_id) =
            create_chat_model_with_model_id(role, &self.config, self.temperature).map_err(
                |error| {
                    TinyAgentsError::Model(format!(
                        "openhuman: no model for workload role `{role}`: {error}"
                    ))
                },
            )?;

        tracing::debug!(
            target: "tinyagents",
            role,
            model_id = %model_id,
            "[tinyagents][model_resolver] built a chat model for this workload role"
        );

        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(role, Arc::clone(&model));
        Ok(model)
    }
}

impl std::fmt::Debug for OpenHumanModelResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenHumanModelResolver")
            .field("temperature", &self.temperature)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<State: Send + Sync> ModelResolver<State> for OpenHumanModelResolver {
    async fn resolve(&self, req: &ModelResolveRequest) -> TaResult<Arc<dyn ChatModel<State>>> {
        let role = workload_role_for(req);
        let inner = self.base_model_for_role(role)?;
        Ok(Arc::new(StatelessModel { inner }))
    }
}

#[cfg(test)]
#[path = "model_resolver_tests.rs"]
mod tests;
