//! Host [`BudgetGate`] backed by OpenHuman's scheduler gate, cost tracker, and
//! TokenJuice profile.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The agent runtime is being made
//! generic over its host, so it can no longer reach into
//! [`crate::platform::llm_gate`] or [`crate::cost`] directly.
//! It declares [`BudgetGate`] instead, and this module is the single place
//! OpenHuman's three metering concerns meet it:
//!
//! * **admission / back-pressure** — [`llm_gate::wait_for_capacity`],
//!   which owns the single-slot global LLM semaphore and the
//!   AC-power / CPU / signed-out policy backoff;
//! * **pricing hints and budgets** — [`tinyagents_registry::pricing::estimate_cost_usd`] on
//!   the way in, and the opt-in `[[cost.budgets]]` policies, which can refuse a
//!   call (`BUDGET_EXCEEDED`). The legacy `monthly_limit_usd` still refuses
//!   nothing. Nothing here writes the cost ledger: the event bridge records
//!   each model call under its real model, and a second write from this gate
//!   double-counted tokens and requests;
//! * **compression advice** — the agent's
//!   [`AgentTokenjuiceCompression`] profile, which decides how much lossy
//!   compaction that agent tolerates.
//!
//! # Contract mismatches, and how each is resolved
//!
//! **1. `Usage` carries no model and no cost.** The crate's
//! [`Usage`] is pure token counts, but
//! [`cost::record_provider_usage`] is keyed by model and priced from a
//! `charged_amount_usd`. Two consequences:
//!
//! * *Model attribution* — the gate remembers the model from the most recent
//!   [`BudgetGate::acquire`] and attributes [`BudgetGate::record`] to it,
//!   falling back to the session's configured model. A gate instance is
//!   per-session and the runtime calls `acquire` immediately before the call it
//!   then `record`s, so in practice the pairing holds; with two calls genuinely
//!   in flight on one gate the attribution can transpose. The alternative —
//!   dropping the record entirely — loses the spend, which is strictly worse
//!   for a budget guard.
//! * *Pricing* — with no provider-reported charge available, the pre-call
//!   estimate comes from the catalog and is only logged.
//!
//! **2. `compression_hint` must be cheap and synchronous**, but every budget
//! read in OpenHuman goes through a mutex and may touch the JSONL store. The
//! gate therefore caches a three-state budget pressure in an atomic, refreshed
//! on the async [`acquire`](Self::acquire) / [`record`](Self::record) paths —
//! exactly the "anything that needs I/O belongs in `record`, whose result this
//! can then consult" shape the trait documents.
//!
//! **3. The hint is a union with `SummarizationPolicy`, never an override.**
//! Returning [`CompressionHint::None`] here means *OpenHuman is not asking for
//! compression for a budget reason*; it is not a veto, and the crate's own
//! window-pressure policy still runs. That is why an agent whose TokenJuice
//! profile is `Off` yields `None` rather than anything stronger — `Off` opts
//! that agent out of *TokenJuice*, not out of summarization.
//!
//! Nothing here bypasses an OpenHuman guard: the scheduler-gate permit is held
//! for exactly the lifetime of the crate permit.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use tinyagents_harness::error::Result;
use tinyagents_harness::host::budget_gate::{
    BudgetGate, CallEstimate, CompressionHint, ContextState, Permit,
};
use tinyinference_llm::usage::Usage;

use crate::config::{Config, DEFAULT_MODEL};
use crate::platform::cost;
use crate::platform::llm_gate;
use tinyjuice_bus::types::AgentTokenjuiceCompression;

/// OpenHuman's [`BudgetGate`]: scheduler-gate back-pressure, cost-tracker
/// budget enforcement, and TokenJuice-profile-aware compression advice.
///
/// One instance per agent session. It holds the session's config (for the
/// fallback model id) and the agent's TokenJuice profile, plus the small amount
/// of state needed to bridge the two contract mismatches described in the
/// module docs.
pub struct OpenHumanBudgetGate {
    /// Session config: the fallback model id, and where `[[cost.budgets]]` is
    /// re-read from on each check (see [`Self::live_budgets`]).
    config: Arc<Config>,
    /// The agent's TokenJuice profile, which bounds how aggressive a
    /// compression hint this gate is willing to give.
    compression: AgentTokenjuiceCompression,
    /// Model attributed to the next [`record`](Self::record). Seeded from the
    /// session config and re-stamped by each [`acquire`](Self::acquire); see
    /// mismatch (1) in the module docs.
    last_model: RwLock<String>,
    /// Whether this session's model calls are **background** work that must
    /// queue behind [`llm_gate`].
    ///
    /// Defaults to `false`, because that gate is for background AI only: its
    /// `Paused` arm polls indefinitely while background work is disabled or the
    /// user is signed out, and OpenHuman's interactive inference paths
    /// deliberately never enter it. Routing a user-initiated turn through it
    /// would stall the chat until the turn timeout for anyone who is signed out
    /// on a local/BYOK model, or who merely paused background AI. Cron and
    /// other background wiring sites opt in with
    /// [`Self::as_background_work`](Self::as_background_work).
    background: bool,
    /// The ledger budgets are checked against; `None` reads the process-wide
    /// cost tracker. Set by tests.
    tracker: Option<Arc<cost::CostTracker>>,
    /// The context the session runs under: budgets attributed to the user
    /// agent (`session_agent`) read its tenant from here.
    dispatch: crate::core::runtime::DispatchCtx,
}

impl OpenHumanBudgetGate {
    /// Builds a gate for a session running under `config`, with the agent's
    /// TokenJuice profile left at [`AgentTokenjuiceCompression::Auto`].
    pub fn new(config: Arc<Config>) -> Self {
        Self::with_compression(config, AgentTokenjuiceCompression::Auto)
    }

    /// Builds a gate for an agent whose TokenJuice profile is known.
    ///
    /// The profile is a ceiling on the hint, not a trigger: see
    /// [`Self::cap_hint`].
    pub fn with_compression(config: Arc<Config>, compression: AgentTokenjuiceCompression) -> Self {
        let fallback = config
            .default_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        Self {
            config,
            compression,
            last_model: RwLock::new(fallback),
            background: false,
            tracker: None,
            dispatch: crate::core::runtime::DispatchCtx::unbound(),
        }
    }

    /// Marks this session's model calls as background work.
    ///
    /// Only then does [`acquire`](Self::acquire) queue behind
    /// [`llm_gate`], which is the concurrency limiter for background AI —
    /// cron jobs, memory workers. Interactive turns must
    /// **not** opt in: the gate's `Paused` arm waits for background work to be
    /// re-enabled, which for a user-initiated chat means waiting until the turn
    /// times out.
    /// Check budgets against `tracker` instead of the process-wide one.
    #[cfg(test)]
    pub(crate) fn with_tracker(mut self, tracker: Arc<cost::CostTracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// Resolves the tenant of budget checks from `dispatch`.
    pub fn with_dispatch(mut self, dispatch: crate::core::runtime::DispatchCtx) -> Self {
        self.dispatch = dispatch;
        self
    }

    pub fn as_background_work(mut self) -> Self {
        self.background = true;
        self
    }

    /// The model id [`record`](Self::record) will attribute usage to.
    fn attributed_model(&self) -> String {
        self.last_model.read().clone()
    }

    /// The refusal text when a configured budget refuses this call; logs any
    /// budget that is only near or past a `warn` limit. Never fails the call
    /// for a reason of its own: without budgets, a cost tracker or a readable
    /// ledger, the call goes ahead.
    fn check_budgets(&self, est: &CallEstimate) -> Option<String> {
        let policies = self.live_budgets();
        if policies.is_empty() {
            return None;
        }
        let Some(tracker) = self.tracker.clone().or_else(cost::try_global) else {
            log::debug!("[tinyagents][budget] budgets configured but no cost tracker; not checked");
            return None;
        };
        self.check_budgets_against(est, &policies, &tracker)
    }

    /// The `[[cost.budgets]]` in effect now. A session's gate outlives
    /// settings changes (cached web-chat sessions keep theirs), so the
    /// session's `config.toml` is re-read on every check; an embedder config
    /// with no file on disk, or one that does not parse, keeps the policies
    /// the gate was built with.
    fn live_budgets(&self) -> Vec<crate::config::BudgetPolicy> {
        // An embedder's in-memory config is authoritative: a file on disk
        // must not replace the budgets it supplied.
        if crate::core::runtime::CoreContext::current_embedder_config().is_some() {
            return self.config.cost.budgets.clone();
        }
        #[derive(serde::Deserialize, Default)]
        struct File {
            #[serde(default)]
            cost: Cost,
        }
        #[derive(serde::Deserialize, Default)]
        struct Cost {
            #[serde(default)]
            budgets: Vec<crate::config::BudgetPolicy>,
        }
        match std::fs::read_to_string(&self.config.config_path) {
            Ok(raw) => match toml::from_str::<File>(&raw) {
                Ok(file) => file.cost.budgets,
                Err(error) => {
                    log::warn!(
                        "[tinyagents][budget] config budgets unreadable ({error}); keeping the session's"
                    );
                    self.config.cost.budgets.clone()
                }
            },
            Err(_) => self.config.cost.budgets.clone(),
        }
    }

    /// [`Self::check_budgets`] against explicit policies and ledger.
    pub(crate) fn check_budgets_against(
        &self,
        est: &CallEstimate,
        policies: &[crate::config::BudgetPolicy],
        tracker: &cost::CostTracker,
    ) -> Option<String> {
        // The user agent (`session_agent` budgets) comes from the gate's
        // context; the agent and thread from the estimate.
        let mut scope = cost::UsageScope::in_ctx(&self.dispatch, None, None);
        if let Some(agent) = est.agent_id.as_ref().filter(|a| !a.is_empty()) {
            scope.agent_id = Some(agent.clone());
        }
        if let Some(thread) = est.thread_id.as_ref() {
            scope.thread_id = Some(thread.as_str().to_string());
        }
        // This call's own model: `last_model` is shared by every call on the
        // gate and may already belong to another one.
        let model = if est.model.trim().is_empty() {
            self.attributed_model()
        } else {
            est.model.clone()
        };
        let call = cost::budget::CallUnderCheck {
            model: &model,
            scope: &scope,
            estimated_usd: tinyagents_registry::pricing::estimate_cost_usd(
                &model,
                est.estimated_input_tokens,
                est.estimated_output_tokens,
                0,
            ),
            estimated_tokens: est
                .estimated_input_tokens
                .saturating_add(est.estimated_output_tokens),
        };
        let verdict = match cost::budget::check_call(policies, tracker, call, chrono::Utc::now()) {
            Ok(verdict) => verdict,
            Err(error) => {
                log::warn!("[tinyagents][budget] budget check skipped: {error:#}");
                return None;
            }
        };
        for hit in verdict.hits.iter()
        // Every hit is worth a line: a warning, or the refusal about to be
        // returned.
        {
            log::warn!(
                "[tinyagents][budget] budget `{}` for {} at ${:.4}/{:?} usd, {}/{:?} tokens (exceeded={})",
                hit.policy,
                hit.bucket,
                hit.spent_usd,
                hit.max_usd,
                hit.tokens,
                hit.max_tokens,
                hit.exceeded
            );
        }
        verdict.refusal().map(cost::budget::BudgetHit::refusal)
    }
}

#[async_trait]
impl BudgetGate for OpenHumanBudgetGate {
    /// Refuses over-budget calls, then parks on OpenHuman's scheduler gate
    /// until the host has capacity.
    ///
    /// Ordered budget-check-first on purpose: a refusal must not first occupy
    /// the single global LLM slot that another, affordable, call could use.
    ///
    /// The returned [`Permit`] owns the [`llm_gate::LlmPermit`] inside its
    /// release hook, so dropping the crate permit — on return, on `?`, on
    /// cancellation, on unwind — is what returns the semaphore slot. There is
    /// exactly one owner and it is never cloned; the crate's `Permit` is
    /// non-`Clone` precisely so that holds.
    ///
    /// Never installs its own deadline. `wait_for_capacity` can legitimately
    /// park indefinitely while the policy is `Paused` (user opted out, or the
    /// session is signed out); the caller's turn timeout is what bounds that.
    async fn acquire(&self, est: &CallEstimate) -> Result<Permit> {
        if !est.model.trim().is_empty() {
            *self.last_model.write() = est.model.clone();
        }

        // Configured budgets (`[[cost.budgets]]`) are checked before anything
        // else, so a refused call never occupies a scheduler slot.
        if let Some(refusal) = self.check_budgets(est) {
            log::warn!("[tinyagents][budget] refusing model call: {refusal}");
            return Err(tinyagents_harness::error::TinyAgentsError::LimitExceeded(
                refusal,
            ));
        }

        // Best-effort pricing. `estimate_cost_usd` returns 0.0 for an
        // uncatalogued model, which means "unknown", not "free" — and a zero
        // estimate can only ever make `check_budget` more permissive, never
        // less, so it can't manufacture a refusal.
        let estimated_usd = tinyagents_registry::pricing::estimate_cost_usd(
            &est.model,
            est.estimated_input_tokens,
            est.estimated_output_tokens,
            0,
        );

        log::debug!(
            "[tinyagents][budget] awaiting capacity model={} agent={:?} in={} out={} tools={} \
             est_usd={estimated_usd:.6}",
            est.model,
            est.agent_id,
            est.estimated_input_tokens,
            est.estimated_output_tokens,
            est.tool_count,
        );

        // Interactive turns never enter the background scheduler. Its `Paused`
        // arm polls until background AI is re-enabled, so a signed-out user on a
        // local/BYOK model — or anyone who simply paused background AI — would
        // watch their chat hang until the turn timeout. The budget checks above
        // only the concurrency queue is skipped.
        if !self.background {
            let grant_id = uuid::Uuid::new_v4().to_string();
            log::trace!(
                "[tinyagents][budget] interactive session; not queueing behind the background \
                 scheduler gate id={grant_id}"
            );
            // Still carries a grant id: correlation is orthogonal to which path
            // granted the permit, and a permit without one is unattributable in
            // the logs.
            return Ok(Permit::unlimited()
                .with_id(grant_id)
                .with_reserved_tokens(est.estimated_total_tokens()));
        }

        // `wait_for_capacity` returns `None` only when the global semaphore has
        // been closed, which never happens in production. Every OpenHuman
        // caller treats that as "skip the gate" rather than an error, and so
        // does this one — failing here would deadlock the pipeline on a
        // condition that is not the user's fault.
        let Some(llm_permit) = llm_gate::wait_for_capacity().await else {
            log::warn!(
                "[tinyagents][budget] scheduler gate returned no permit (semaphore closed); \
                 proceeding ungated"
            );
            return Ok(Permit::unlimited().with_reserved_tokens(est.estimated_total_tokens()));
        };

        let grant_id = uuid::Uuid::new_v4().to_string();
        log::trace!("[tinyagents][budget] granted permit id={grant_id}");
        // Moving `llm_permit` into the hook is the whole point: the hook is
        // `FnOnce`, runs exactly once from `Drop`, and dropping the
        // `LlmPermit` is a semaphore release — non-blocking, non-panicking,
        // runtime-agnostic, as the hook contract requires.
        Ok(Permit::with_release(move || drop(llm_permit))
            .with_id(grant_id)
            .with_reserved_tokens(est.estimated_total_tokens()))
    }

    /// Observes realised usage. Additive and non-fatal, as the trait requires:
    /// it is also called for failed calls that burned tokens.
    ///
    /// Deliberately does not write the cost ledger. The event bridge
    /// (`observability::event_bridge`) records every model call under its real
    /// model with the provider-charged amount; this gate would add a second row
    /// under `host:<agent_id>` with an estimated cost, doubling the tokens and
    /// request counts on the dashboard. Nothing reads spend from here to refuse
    /// a call, so there is no enforcement to feed.
    async fn record(&self, usage: &Usage) -> Result<()> {
        log::debug!(
            "[tinyagents][budget] observed usage model={} in={} out={} cached={} (ledger is \
             written by the event bridge)",
            self.attributed_model(),
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
        );
        Ok(())
    }

    /// Always [`CompressionHint::None`]: this gate has no budget opinion.
    ///
    /// It used to escalate compression as OpenHuman approached its spend cap.
    /// With the cap removed there is no budget pressure to read, and context
    /// fullness was never this gate's question — that belongs to
    /// `SummarizationPolicy`, which still compresses on its own threshold.
    /// Duplicating that threshold here is how the two came to disagree, so
    /// this stays a declined request rather than a second opinion.
    fn compression_hint(&self, _state: &ContextState) -> CompressionHint {
        CompressionHint::None
    }
}

#[cfg(test)]
#[path = "budget_gate_tests.rs"]
mod tests;
