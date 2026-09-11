//! Host [`BudgetGate`] backed by OpenHuman's scheduler gate, cost tracker, and
//! TokenJuice profile.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The agent runtime is being made
//! generic over its host, so it can no longer reach into
//! [`crate::openhuman::cron::scheduler_gate`] or [`crate::openhuman::cost`] directly.
//! It declares [`BudgetGate`] instead, and this module is the single place
//! OpenHuman's three metering concerns meet it:
//!
//! * **admission / back-pressure** — [`scheduler_gate::wait_for_capacity`],
//!   which owns the single-slot global LLM semaphore and the
//!   AC-power / CPU / signed-out policy backoff;
//! * **budget refusal + accounting** — [`cost::CostTracker::check_budget`] and
//!   [`cost::record_provider_usage`], priced through
//!   [`cost::catalog::estimate_cost_usd`];
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
//! * *Pricing* — with no provider-reported charge available, the cost is
//!   estimated from the catalog. See the `TODO(phase4)` on [`Self::record`]:
//!   OpenHuman will label that record `CostSource::ProviderCharged` even though
//!   it is an estimate.
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
//! Nothing here bypasses an OpenHuman guard: budget refusal still goes through
//! `check_budget` (which honours `cost.enabled`), and the scheduler-gate permit
//! is held for exactly the lifetime of the crate permit.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use tinyagents_harness::error::{Result, TinyAgentsError};
use tinyagents_harness::host::budget_gate::{
    BudgetGate, CallEstimate, CompressionHint, ContextState, Permit,
};
use tinyinference::usage::Usage;

use crate::openhuman::config::{Config, DEFAULT_MODEL};
use crate::openhuman::cron::scheduler_gate;
use crate::openhuman::inference::provider::types::UsageInfo;
use crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression;
use crate::openhuman::platform::cost::{self, BudgetCheck};

/// Cached budget pressure, encoded for [`AtomicU8`].
///
/// A plain enum behind a lock would make [`BudgetGate::compression_hint`] —
/// which the runtime calls between every iteration of a turn — take a lock on
/// the hot path. An atomic load is what the trait's "read a counter, compare
/// against a threshold" wording asks for.
const PRESSURE_NORMAL: u8 = 0;
/// The warn threshold (`cost.warn_at_percent`) has been crossed.
const PRESSURE_WARNING: u8 = 1;
/// A daily or monthly limit has been reached. Set only transiently: an
/// `Exceeded` check also refuses the call it was made for.
const PRESSURE_EXCEEDED: u8 = 2;

/// Context utilization at which an *already budget-driven* soft hint is
/// escalated to hard.
///
/// Deliberately only an escalator. Utilization on its own never originates a
/// hint here — that question belongs to the crate's `SummarizationPolicy`, and
/// answering it a second time from the host is how the two silently disagree.
const ESCALATE_AT_UTILIZATION: f64 = 0.9;

/// OpenHuman's [`BudgetGate`]: scheduler-gate back-pressure, cost-tracker
/// budget enforcement, and TokenJuice-profile-aware compression advice.
///
/// One instance per agent session. It holds the session's config (for the
/// fallback model id) and the agent's TokenJuice profile, plus the small amount
/// of state needed to bridge the two contract mismatches described in the
/// module docs.
pub struct OpenHumanBudgetGate {
    /// Session config. Read only for the fallback model id — everything
    /// budget-shaped is read live from the global cost tracker so a settings
    /// update takes effect without rebuilding the gate.
    config: Arc<Config>,
    /// The agent's TokenJuice profile, which bounds how aggressive a
    /// compression hint this gate is willing to give.
    compression: AgentTokenjuiceCompression,
    /// Model attributed to the next [`record`](Self::record). Seeded from the
    /// session config and re-stamped by each [`acquire`](Self::acquire); see
    /// mismatch (1) in the module docs.
    last_model: RwLock<String>,
    /// Cached budget pressure; one of the `PRESSURE_*` constants.
    pressure: AtomicU8,
    /// Whether this session's model calls are **background** work that must
    /// queue behind [`scheduler_gate`].
    ///
    /// Defaults to `false`, because that gate is for background AI only: its
    /// `Paused` arm polls indefinitely while background work is disabled or the
    /// user is signed out, and OpenHuman's interactive inference paths
    /// deliberately never enter it. Routing a user-initiated turn through it
    /// would stall the chat until the turn timeout for anyone who is signed out
    /// on a local/BYOK model, or who merely paused background AI. Cron and
    /// subconscious wiring sites opt in with
    /// [`Self::as_background_work`](Self::as_background_work).
    background: bool,
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
            pressure: AtomicU8::new(PRESSURE_NORMAL),
            background: false,
        }
    }

    /// Marks this session's model calls as background work.
    ///
    /// Only then does [`acquire`](Self::acquire) queue behind
    /// [`scheduler_gate`], which is the concurrency limiter for background AI —
    /// cron jobs, the subconscious tick, memory workers. Interactive turns must
    /// **not** opt in: the gate's `Paused` arm waits for background work to be
    /// re-enabled, which for a user-initiated chat means waiting until the turn
    /// times out.
    pub fn as_background_work(mut self) -> Self {
        self.background = true;
        self
    }

    /// The model id [`record`](Self::record) will attribute usage to.
    fn attributed_model(&self) -> String {
        self.last_model.read().clone()
    }

    /// Re-reads the budget and caches the resulting pressure.
    ///
    /// `pending_usd` is the cost of a call about to be made (`0.0` when
    /// reconciling after one). Returns the raw [`BudgetCheck`] so
    /// [`acquire`](Self::acquire) can refuse on `Exceeded`; returns `None` when
    /// the tracker is uninitialised (before bootstrap, and in unit tests),
    /// which OpenHuman treats everywhere as "no budget opinion", never as a
    /// refusal.
    fn refresh_pressure(&self, pending_usd: f64) -> Option<BudgetCheck> {
        let tracker = cost::try_global()?;
        match tracker.check_budget(pending_usd) {
            Ok(check) => {
                let pressure = match check {
                    BudgetCheck::Allowed => PRESSURE_NORMAL,
                    BudgetCheck::Warning { .. } => PRESSURE_WARNING,
                    BudgetCheck::Exceeded { .. } => PRESSURE_EXCEEDED,
                };
                self.pressure.store(pressure, Ordering::Relaxed);
                Some(check)
            }
            Err(err) => {
                // A failed budget read is not a refusal: `check_budget` errors
                // on a malformed estimate or a storage problem, neither of
                // which is evidence the user is over budget. Refusing here
                // would turn a bad JSONL file into "no agent may run".
                log::warn!(
                    "[tinyagents][budget] check_budget failed; proceeding without \
                     a budget opinion: {err}"
                );
                None
            }
        }
    }

    /// Lowers a hint to what the agent's TokenJuice profile tolerates.
    ///
    /// * `Off` — the agent has opted out of TokenJuice, so this gate asks for
    ///   nothing. Union semantics mean the crate's own policy still compresses
    ///   when the window demands it; this is a declined request, not a veto.
    /// * `Light` — non-lossy reductions only, so `Hard` (which invites lossy
    ///   compaction) is softened to `Soft`.
    /// * `Auto` / `Full` — pass through. TokenJuice itself treats `Auto` as
    ///   `Full` for callers that have not resolved it.
    fn cap_hint(&self, hint: CompressionHint) -> CompressionHint {
        match self.compression {
            AgentTokenjuiceCompression::Off => CompressionHint::None,
            AgentTokenjuiceCompression::Light => match hint {
                CompressionHint::Hard => CompressionHint::Soft,
                other => other,
            },
            AgentTokenjuiceCompression::Auto | AgentTokenjuiceCompression::Full => hint,
        }
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
    /// The returned [`Permit`] owns the [`scheduler_gate::LlmPermit`] inside its
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

        // Best-effort pricing. `estimate_cost_usd` returns 0.0 for an
        // uncatalogued model, which means "unknown", not "free" — and a zero
        // estimate can only ever make `check_budget` more permissive, never
        // less, so it can't manufacture a refusal.
        let estimated_usd = cost::catalog::estimate_cost_usd(
            &est.model,
            est.estimated_input_tokens,
            est.estimated_output_tokens,
            0,
        );

        if let Some(BudgetCheck::Exceeded {
            current_usd,
            limit_usd,
            period,
        }) = self.refresh_pressure(estimated_usd)
        {
            log::warn!(
                "[tinyagents][budget] refusing model call: {period:?} spend ${current_usd:.4} \
                 of ${limit_usd:.4} limit (model={} est=${estimated_usd:.4})",
                est.model
            );
            return Err(TinyAgentsError::LimitExceeded(format!(
                "cost budget exceeded: {period:?} spend ${current_usd:.4} of ${limit_usd:.4} limit"
            )));
        }

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
        // still apply either way; only the concurrency queue is skipped.
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
        let Some(llm_permit) = scheduler_gate::wait_for_capacity().await else {
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

    /// Persists realised usage to OpenHuman's cost tracker and refreshes the
    /// cached budget pressure.
    ///
    /// Additive and non-fatal, as the trait requires: it is also called for
    /// failed calls that burned tokens, and `record_provider_usage` swallows
    /// its own write errors so cost tracking can never break a turn. An
    /// all-zero `Usage` is skipped by that helper rather than inflating the
    /// request count with a non-event.
    ///
    /// TODO(phase4): the record's `CostSource` will read `ProviderCharged`
    /// even though the amount is a catalog estimate — `build_token_usage` in
    /// `src/openhuman/cost/global.rs` infers provenance from
    /// `charged_amount_usd > 0.0`, and the crate's `Usage` carries neither a
    /// charged amount nor a cost field to distinguish them. The fix is an
    /// explicit estimated-usage entry point in `cost::global` (or a
    /// `cost_source` argument on `record_provider_usage`); pricing here is
    /// deliberately not skipped, because a zero-cost ledger would silently
    /// disable `check_budget` enforcement altogether.
    async fn record(&self, usage: &Usage) -> Result<()> {
        let model = self.attributed_model();
        let charged_amount_usd = cost::catalog::estimate_cost_usd(
            &model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
        );

        cost::record_provider_usage(
            &model,
            &UsageInfo {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                // The crate's `Usage` does not carry the model's context
                // window; `UsageInfo::context_window` documents 0 as unknown.
                context_window: 0,
                cached_input_tokens: usage.cache_read_tokens,
                cache_creation_tokens: usage.cache_creation_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                charged_amount_usd,
            },
        );

        log::debug!(
            "[tinyagents][budget] recorded usage model={model} in={} out={} cached={} \
             est_usd={charged_amount_usd:.6}",
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
        );

        // Reconcile after the spend: this is the I/O-bearing refresh that
        // `compression_hint` then reads for free.
        self.refresh_pressure(0.0);
        Ok(())
    }

    /// Advises compression when OpenHuman is under *budget* pressure.
    ///
    /// A single relaxed atomic load plus, at most, one float compare — no lock,
    /// no I/O, safe to call between every iteration of a turn.
    ///
    /// Context fullness never originates a hint here. That question is
    /// `SummarizationPolicy`'s, and duplicating its threshold is how the two
    /// come to disagree; utilization is used only to escalate a hint the budget
    /// already justified. Consequently a healthy budget yields
    /// [`CompressionHint::None`] — this gate declining to *ask*, with the
    /// crate's own policy still free to compress.
    fn compression_hint(&self, state: &ContextState) -> CompressionHint {
        let hint = match self.pressure.load(Ordering::Relaxed) {
            PRESSURE_WARNING => {
                let crowded = state
                    .utilization()
                    .is_some_and(|used| used >= ESCALATE_AT_UTILIZATION);
                if crowded {
                    CompressionHint::Hard
                } else {
                    CompressionHint::Soft
                }
            }
            PRESSURE_EXCEEDED => CompressionHint::Hard,
            _ => CompressionHint::None,
        };
        self.cap_hint(hint)
    }
}

#[cfg(test)]
#[path = "budget_gate_tests.rs"]
mod tests;
