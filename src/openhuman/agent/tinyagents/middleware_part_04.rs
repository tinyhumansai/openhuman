
/// `before_model`: enforce OpenHuman's daily/monthly cost budgets **before** a
/// model call spends (issue #4249, Phase 5). Reads the global
/// [`CostTracker`](crate::openhuman::platform::cost) and, when cost budgets are configured
/// and already exceeded, fails the run before the provider call; a warning
/// threshold logs but proceeds. This enforcement path stays **authoritative**.
///
/// Self-gating: a no-op unless a global tracker exists and `config.enabled` with
/// a limit is set (`check_budget` returns `Allowed` otherwise). Complements the
/// post-call `StopHookMiddleware` per-turn USD cap. Projecting the *next* call's
/// cost pre-spend (vs the already-exceeded check here) needs an input-token
/// estimate — a follow-up.
///
/// # Shadow role (W2-budget-dedupe)
///
/// When built with [`with_shadow`](Self::with_shadow), this middleware is ALSO a
/// divergence-logging shadow over the observe-only crate
/// [`BudgetMiddleware`](tinyagents_harness::middleware::BudgetMiddleware). It
/// keeps enforcing exactly as before, but at `after_agent` it compares the
/// crate `BudgetMiddleware`'s shared [`BudgetTracker`] accumulation against the
/// authoritative runtime [`AgentRun::usage`] and logs `[budget_shadow]` parity
/// or divergence (compact numeric summary; no PII). Both accumulate the same
/// per-call `response.usage`, so token totals must match once the crate
/// middleware is on the path — this is the parity signal that must be clean
/// before enforcement can flip to the crate owner (see the flip-criteria comment
/// at the registration site in `tinyagents/mod.rs`). Cost is intentionally NOT
/// compared: the observe-only crate middleware has no pricing table, so its cost
/// stays zero while the local path prices via `cost::catalog` — cost parity is a
/// flip-criteria follow-up.
pub(crate) struct CostBudgetMiddleware {
    /// Observe-only crate `BudgetMiddleware`'s shared tracker handle, for the
    /// end-of-run `[budget_shadow]` comparison. `None` when the shadow is not
    /// installed (isolated unit tests of the enforcement gate).
    shadow_tracker: Option<BudgetTracker>,
}

impl CostBudgetMiddleware {
    /// Enforcement-only gate with no shadow comparison (isolated unit tests).
    pub(crate) fn new() -> Self {
        Self {
            shadow_tracker: None,
        }
    }

    /// Enforcement gate that ALSO compares its per-run token accounting against
    /// the observe-only crate `BudgetMiddleware`'s shared `tracker` at end of run
    /// and logs `[budget_shadow]` parity/divergence.
    pub(crate) fn with_shadow(tracker: BudgetTracker) -> Self {
        Self {
            shadow_tracker: Some(tracker),
        }
    }
}

#[async_trait]
impl Middleware<()> for CostBudgetMiddleware {
    fn name(&self) -> &str {
        "cost_budget"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        use crate::openhuman::platform::cost::types::BudgetCheck;
        let Some(tracker) = crate::openhuman::platform::cost::try_global() else {
            return Ok(());
        };

        // #5016: exempt the CURRENT request when it is BYOK, not just BYOK
        // history. Excluding BYOK from the managed totals alone still refuses a
        // mixed-route user's own-key calls once their managed spend has
        // legitimately crossed the cap — managed exhaustion would disable the
        // provider OpenHuman never bills for, which is the whole bug. Classify
        // this call's route and skip the gate when OpenHuman is not the biller.
        if let Some(model) = request.model.as_deref() {
            let route = crate::openhuman::platform::cost::route::route_for_model(model);
            if !route.counts_toward_budget() {
                tracing::debug!(
                    %model,
                    ?route,
                    "[tinyagents::mw] BYOK/local route — skipping the managed budget gate (#5016)"
                );
                return Ok(());
            }
        }

        // Pass 0.0 to test whether we are *already* over budget before spending
        // more (rather than projecting this call's cost, which needs a token
        // estimate).
        match tracker.check_budget(0.0) {
            Ok(BudgetCheck::Exceeded {
                current_usd,
                limit_usd,
                period,
            }) => {
                tracing::warn!(
                    %current_usd, %limit_usd, ?period,
                    "[tinyagents::mw] cost budget exceeded — failing before model call"
                );
                Err(tinyagents_harness::TinyAgentsError::LimitExceeded(format!(
                    "cost budget exceeded: {period:?} spend ${current_usd:.4} \u{2265} limit ${limit_usd:.4}"
                )))
            }
            Ok(BudgetCheck::Warning {
                current_usd,
                limit_usd,
                period,
            }) => {
                tracing::warn!(
                    %current_usd, %limit_usd, ?period,
                    "[tinyagents::mw] cost budget warning threshold reached"
                );
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Shadow parity check (W2-budget-dedupe). Enforcement already happened per
    /// call in `before_model`; here we only observe. Compares the observe-only
    /// crate `BudgetMiddleware`'s accumulated token spend against the runtime's
    /// authoritative `AgentRun::usage` and logs `[budget_shadow]` divergence.
    /// Never fails the run.
    async fn after_agent(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        run: &mut AgentRun,
    ) -> TaResult<()> {
        let Some(tracker) = &self.shadow_tracker else {
            return Ok(());
        };
        let crate_usage = tracker.snapshot().usage; // UsageTotals (crate shadow)
        let local = run.usage; // UsageTotals (runtime authoritative)
        let l = &local.usage;
        let c = &crate_usage.usage;
        let diverged = l.input_tokens != c.input_tokens
            || l.output_tokens != c.output_tokens
            || l.cache_read_tokens != c.cache_read_tokens
            || l.total_tokens != c.total_tokens
            || local.calls != crate_usage.calls;
        if diverged {
            tracing::warn!(
                local_calls = local.calls,
                crate_calls = crate_usage.calls,
                local_in = l.input_tokens,
                crate_in = c.input_tokens,
                local_out = l.output_tokens,
                crate_out = c.output_tokens,
                local_cached = l.cache_read_tokens,
                crate_cached = c.cache_read_tokens,
                local_total = l.total_tokens,
                crate_total = c.total_tokens,
                "[budget_shadow] divergence: crate BudgetMiddleware token accounting differs from authoritative AgentRun.usage"
            );
        } else {
            tracing::debug!(
                calls = local.calls,
                input = l.input_tokens,
                output = l.output_tokens,
                cached = l.cache_read_tokens,
                total = l.total_tokens,
                "[budget_shadow] parity: crate BudgetMiddleware token accounting matches AgentRun.usage"
            );
        }
        Ok(())
    }
}

/// `after_tool`: stop (or nudge) the run when tool calls keep failing with no
/// progress (issue #4249). The legacy tool loop's progress guard surfaced a
/// root-cause halt summary — a security/approval denial re-issued unchanged, an
/// identical error retried, or *different* commands all failing — instead of
/// burning the whole iteration budget and ending on a generic cap error. The
/// tinyagents path kept only the model/tool call caps, so this reinstates the
/// guard as a graph middleware.
///
/// As of tinyagents 1.5.0 the escalation ladder itself lives in the crate
/// ([`NoProgressTracker`], extracted upstream from OpenHuman #4389). This
/// middleware is now a **thin driver**: it captures the per-call argument
/// fingerprint (the tool result carries no arguments), feeds each outcome into
/// [`NoProgressTracker::record`], and lowers the returned [`NoProgress`] verdict
/// into OpenHuman steering. It owns only the OpenHuman-side policy:
///
/// - [`NoProgress::Continue`] — do nothing.
/// - [`NoProgress::Nudge`] — inject the crate's structured "no progress since
///   step X" corrective into the working transcript via
///   [`SteeringCommand::InjectMessage`] so the next model call sees it and
///   changes strategy *before* the same-strategy retry cap trips. (Not
///   `Redirect`: that verb is outside the Interactive steering allowlist and
///   would abort the turn — see the nudge call site.)
/// - [`NoProgress::Halt`] — record the crate's root-cause summary into the shared
///   [`HaltSummarySlot`](super::HaltSummarySlot) (the turn overrides its final
///   text with it) and pause the run via the shared steering handle (same
///   mechanism as the stop-hook / cap pausers), then [`reset`](NoProgressTracker::reset)
///   so a resumed run does not immediately re-pause on the latched state.
pub(crate) struct RepeatedToolFailureMiddleware {
    handle: SteeringHandle,
    halt_summary: super::HaltSummarySlot,
    /// Crate no-progress escalation ladder — the single source of the
    /// identical-failure / varied-failure / hard-reject logic (tinyagents 1.5.0).
    tracker: NoProgressTracker,
    /// Monotonic tool-outcome counter, used only for the crate's "no progress
    /// since step X" nudge wording. Not the model-call count, but a stable,
    /// increasing marker is all the wording needs.
    step: AtomicUsize,
    /// call_id → argument fingerprint, captured in `before_tool` (the tool result
    /// carries no arguments). Folded into the identical-repeat signature so the
    /// "identical arguments" halt only trips on the *same* args — two different
    /// argument sets that happen to share a first error line don't count as a
    /// repeat and can't pre-empt the generic no-progress backstop.
    arg_sigs: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Recoverable-failure ladder (issue #4463): transient failures (timeouts,
    /// connection resets, rate limits, 5xx) are routed here instead of the crate
    /// tracker so they get the legacy extended headroom
    /// ([`RECOVERABLE_REPEAT_FAILURE_THRESHOLD`] identical /
    /// [`RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`] consecutive) rather than the
    /// crate's fixed 3/6, which is right only for deterministic failures.
    /// `tool\u{1f}args` → identical-failure count; persists across the turn.
    recoverable_sig_counts: std::sync::Mutex<std::collections::HashMap<String, u32>>,
    /// Consecutive recoverable-looking failures with no success in between. Reset
    /// on any success or non-recoverable failure (mirrors the legacy guard).
    recoverable_consecutive: AtomicU32,
}

impl RepeatedToolFailureMiddleware {
    /// Build the breaker. `identical_threshold` (the identical-signature retry
    /// ceiling) is handed straight to [`NoProgressTracker::new`], which clamps it
    /// so a nudge always precedes a halt (a single failure is never a loop).
    pub(crate) fn new(
        handle: SteeringHandle,
        identical_threshold: usize,
        halt_summary: super::HaltSummarySlot,
    ) -> Self {
        Self {
            handle,
            halt_summary,
            tracker: NoProgressTracker::new(identical_threshold),
            step: AtomicUsize::new(0),
            arg_sigs: std::sync::Mutex::new(std::collections::HashMap::new()),
            recoverable_sig_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
            recoverable_consecutive: AtomicU32::new(0),
        }
    }

    /// Clear the consecutive recoverable-failure streak. Called on any success or
    /// non-recoverable failure (the per-signature identical counts persist across
    /// the turn, matching the legacy guard). Idempotent.
    fn reset_recoverable_streak(&self) {
        self.recoverable_consecutive.store(0, Ordering::SeqCst);
    }

    /// Record one recoverable failure and return a root-cause halt summary once
    /// its extended headroom is exhausted (identical `>=` [`RECOVERABLE_REPEAT_FAILURE_THRESHOLD`]
    /// or consecutive `>=` [`RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`]).
    fn record_recoverable(&self, tool: &str, arg_fp: &str, failure_text: &str) -> Option<String> {
        let key = format!("{tool}\u{1f}{arg_fp}");
        let count = self
            .recoverable_sig_counts
            .lock()
            .ok()
            .map(|mut counts| {
                let c = counts.entry(key).or_insert(0);
                *c += 1;
                *c
            })
            .unwrap_or(0);
        let consecutive = self.recoverable_consecutive.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::debug!(
            tool,
            count,
            consecutive,
            "[tinyagents::mw] recoverable tool failure recorded with extended circuit-breaker headroom"
        );
        if count >= RECOVERABLE_REPEAT_FAILURE_THRESHOLD {
            return Some(recoverable_identical_halt_summary(
                tool,
                count,
                failure_text,
            ));
        }
        if consecutive >= RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD {
            return Some(recoverable_no_progress_halt_summary(
                consecutive,
                tool,
                failure_text,
            ));
        }
        None
    }
}

/// Recognise a **user-actionable** blocker in a failing tool result — one only
/// the user can clear — and phrase the halt as a direct ask instead of the
/// crate's generic "the goal looks unreachable in this environment, report this
/// back" summary (issue #4092). Today that's a missing service connection (the
/// issue's canonical example: acting on a service that isn't connected). Such a
/// failure will never self-resolve by retrying, and the fix is the user's, so
/// escalate with a concrete next step instead of looping or reporting a generic
/// dead-end. Returns `None` for failures that are not user-actionable, leaving
/// the crate's summary in place.
fn user_actionable_escalation(tool: &str, error: &str) -> Option<String> {
    let lower = error.to_lowercase();
    let permission_or_scope_failure = lower.contains("[composio:error:insufficient_scope]")
        || lower.contains("[composio:error:trigger_permission]")
        || lower.contains("insufficient scope")
        || lower.contains("insufficient authentication scopes")
        || lower.contains("insufficient permissions")
        || lower.contains("missing required permissions")
        || lower.contains("permission to manage triggers");
    if permission_or_scope_failure {
        return None;
    }
    // Keep this narrow: some scope/permission failures legitimately tell the
    // user to reconnect in Connections, but they are not missing connections.
    let missing_connection = lower.contains("[composio:error:composio_platform]")
        || lower.contains("not connected")
        || lower.contains("isn't connected")
        || lower.contains("is not connected")
        || lower.contains("not enabled")
        || lower.contains("token revoked")
        || lower.contains("connection error, try to authenticate");
    if !missing_connection {
        return None;
    }
    Some(format!(
        "I can't continue without your input: the `{tool}` action needs a service that isn't \
         connected. {}\n\nConnect it (Connections), then tell me to retry — or \
         tell me how you'd like to proceed instead.",
        crate::openhuman::util::truncate_with_ellipsis(error, 400),
    ))
}

/// A stable, bounded fingerprint of a tool call's arguments for the identical-
/// repeat signature (hashed so a huge payload doesn't bloat the map/comparison).
fn args_fingerprint(arguments: &serde_json::Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    arguments.to_string().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Detect a **body-level** failure from `validate_workflow` / `dry_run_workflow`
/// (issue: flows breaker doesn't see repeated invalid-graph loops). Both tools
/// report an invalid graph / aborted sandbox run via `ToolResult::success` with
/// a JSON body carrying top-level `"ok": false`
/// (`src/openhuman/flows/builder_tools.rs`) rather than `ToolResult::error` — so
/// `result.error` stays `None` and the no-progress breaker below never counts
/// the repeat as a failure, letting a graph the model can't fix burn the whole
/// iteration budget instead of tripping the same nudge/halt ladder.
///
/// Scoped to exactly these two tool names: a generic `"ok": false` in some other
/// tool's JSON body may be legitimate data (not a failure signal), so this must
/// not reinterpret arbitrary tool output. Tolerant of non-JSON or missing `ok`
/// content — returns `false` rather than guessing.
fn is_body_level_failure(name: &str, content: &str) -> bool {
    if name != "validate_workflow" && name != "dry_run_workflow" {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(serde_json::Value::Object(map)) => {
            matches!(map.get("ok"), Some(serde_json::Value::Bool(false)))
        }
        _ => false,
    }
}

#[async_trait]
impl Middleware<()> for RepeatedToolFailureMiddleware {
    fn name(&self) -> &str {
        "repeated_tool_failure"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // The tool result carries no arguments, so capture a fingerprint here and
        // correlate it by call_id in `after_tool`.
        if let Ok(mut sigs) = self.arg_sigs.lock() {
            sigs.insert(call.id.clone(), args_fingerprint(&call.arguments));
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let arg_fp = self
            .arg_sigs
            .lock()
            .ok()
            .and_then(|mut sigs| sigs.remove(&result.call_id))
            .unwrap_or_default();
        let step = self.step.fetch_add(1, Ordering::SeqCst) + 1;

        // Body-level failure signal: `validate_workflow` / `dry_run_workflow`
        // report an invalid graph via a `success` result whose JSON body carries
        // `"ok": false` — see `is_body_level_failure`. Only meaningful when
        // `result.error` is `None`; when both are set, `result.error` already
        // drives every check below, so this never double-counts one failure.
        let body_level_failure =
            result.error.is_none() && is_body_level_failure(&result.name, &result.content);

        // Combined failure text for classification: the model-facing content plus
        // the (redundant but authoritative) error field. Both are scanned for the
        // policy / terminal-inference / recoverable markers below.
        let failure_text = match result.error.as_deref() {
            Some(err) => format!("{}\n{}", result.content, err),
            None if body_level_failure => result.content.clone(),
            None => String::new(),
        };

        // ── Part 5 (#3104): terminal delegated-inference fast-halt ──────────────
        // A permanent inference failure (out of budget / provider-config rejection)
        // surfaced by a delegated sub-agent cannot be recovered by retrying — the
        // budget is account-wide and the model/provider config is shared by every
        // (sub-)agent. Halt on the FIRST occurrence with an actionable root cause,
        // *before* the count-based thresholds, because the orchestrator otherwise
        // re-emits the doomed step under varied delegation-tool names so the
        // identical-retry threshold never trips in time.
        if result.error.is_some() {
            if let Some(kind) = terminal_inference_failure_kind(&failure_text) {
                tracing::warn!(
                    tool = %result.name,
                    kind = ?kind,
                    "[tinyagents::mw] terminal delegated-inference failure — halting on first occurrence with root cause"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(terminal_inference_halt_summary(
                        kind,
                        &result.name,
                        &failure_text,
                    ));
                }
                self.handle.send(SteeringCommand::Pause);
                self.tracker.reset();
                self.reset_recoverable_streak();
                return Ok(());
            }
        }

        // A hard policy rejection is marked in the tool output; it can never
        // succeed when re-issued unchanged, so the crate ladder trips it faster
        // (its `HARD_REJECT_HALT_THRESHOLD` of 2). Both the read-only/forbidden
        // block (`POLICY_BLOCKED_MARKER`) and the approval denial / TTL expiry
        // (`POLICY_DENIED_MARKER`) are deterministic — restore the 2-repeat
        // fast-trip for BOTH (issue #4463 part 6: denied had drifted to the
        // generic 3).
        let policy_marked = |s: &str| {
            s.contains(crate::openhuman::security::POLICY_BLOCKED_MARKER)
                || s.contains(crate::openhuman::security::POLICY_DENIED_MARKER)
        };
        let hard_reject =
            policy_marked(&result.content) || result.error.as_deref().is_some_and(policy_marked);

        // ── Part 4: recoverable-failure headroom ────────────────────────────────
        // Transient failures (timeouts, connection resets, rate limits, 5xx) get
        // the legacy extended headroom instead of the crate's deterministic 3/6.
        // Route them to the recoverable ladder; a success or a non-recoverable
        // failure resets that streak and feeds the crate tracker as before.
        let recoverable = result.error.is_some()
            && !hard_reject
            && (is_recoverable_tool_failure(&failure_text)
                || matches!(
                    crate::openhuman::tools::status::classify(&failure_text, false).class,
                    crate::openhuman::tools::status::ToolFailureClass::Timeout
                        | crate::openhuman::tools::status::ToolFailureClass::ServiceUnavailable
                        | crate::openhuman::tools::status::ToolFailureClass::ModelConnection
                ));
        if recoverable {
            // A poll tool's contract is the identical repeat (see
            // [`is_repeat_call_exempt`]), and the thing it repeats on is a
            // *timeout* — which lands here as a recoverable failure. Counting
            // those toward the identical-argument headroom halts exactly the
            // loop the tool is documented to ask for: a sub-agent that outlives
            // eight wait windows killed the turn, discarding work it had already
            // done. `RepeatProgressMiddleware` already honours this exemption on
            // the success side; the failure ladder must agree, or the exemption
            // only holds while the wait happens to return early.
            if is_repeat_call_exempt(&result.name) {
                return Ok(());
            }
            if let Some(summary) = self.record_recoverable(&result.name, &arg_fp, &failure_text) {
                tracing::warn!(
                    tool = %result.name,
                    "[tinyagents::mw] recoverable-failure headroom exhausted — halting run so the root cause surfaces"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(summary);
                }
                self.handle.send(SteeringCommand::Pause);
                self.reset_recoverable_streak();
            }
            // Recoverable failures never feed the crate tracker — its fixed 3/6
            // backstop would halt them before the extended headroom is spent.
            return Ok(());
        }
        // Success or non-recoverable failure: clear the recoverable streak (its
        // per-signature counts persist across the turn) before the crate tracker
        // handles the deterministic 3/6 + hard-reject-2 path below.
        self.reset_recoverable_streak();

        // Union the body-level `ok:false` signal with the existing `error.is_some()`
        // predicate so the crate tracker (which reads `attempt.error` as its sole
        // success/failure signal — `None` means "progress was made, reset every
        // counter") sees the repeat as a failure and feeds it into the same
        // nudge/halt ladder as a real tool error.
        let attempt_error: Option<&str> = match result.error.as_deref() {
            Some(err) => Some(err),
            None if body_level_failure => Some(failure_text.as_str()),
            None => None,
        };
        let attempt = ToolAttempt {
            tool: &result.name,
            arg_fingerprint: &arg_fp,
            error: attempt_error,
            hard_reject,
            // The unknown-tool recovery sentinel is a C3 concern; today every
            // failure feeds the generic backstop exactly as the legacy ladder did.
            recoverable_miss: false,
        };

        match self.tracker.record(step, &attempt) {
            NoProgress::Continue => {}
            NoProgress::Nudge(instruction) => {
                tracing::warn!(
                    tool = %result.name,
                    step,
                    hard_reject,
                    "[tinyagents::mw] no-progress nudge — steering the model to change strategy before the retry cap"
                );
                // Inject the crate's structured corrective as a system message via
                // the `InjectMessage` steering lane. This runs on *every* turn,
                // including the user's live interactive turn, whose steering policy
                // permits only `InjectMessage`/`Pause` — `Redirect` is Background
                // (sub-agent) only, so sending it here aborted every interactive
                // turn that hit the nudge with `steering command redirect is not
                // permitted by the run policy` (a #4473 migration regression). The
                // corrective is trusted, system-generated advisory text, so the
                // `InjectMessage` lane is both permitted and semantically correct.
                self.handle
                    .send(SteeringCommand::InjectMessage(TaMessage::system(
                        instruction,
                    )));
            }
            NoProgress::Halt(summary) => {
                // #4092: if the blocker is user-actionable (a missing connection),
                // escalate with a concrete ask instead of the crate's generic
                // "unreachable environment, report back" summary.
                let escalation = user_actionable_escalation(
                    &result.name,
                    result.error.as_deref().unwrap_or(result.content.as_str()),
                );
                let user_actionable = escalation.is_some();
                let summary = escalation.unwrap_or(summary);
                tracing::warn!(
                    tool = %result.name,
                    step,
                    hard_reject,
                    user_actionable,
                    "[tinyagents::mw] repeated tool failure — halting run so the root cause surfaces"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(summary);
                }
                // Pause at the top of the next iteration (before the next model
                // call), matching the stop-hook / cap pause path. Reset so a
                // resumed run does not immediately re-pause on the latched state
                // (the crate also resets internally on a halt; this is explicit
                // and idempotent).
                self.handle.send(SteeringCommand::Pause);
                self.tracker.reset();
            }
        }
        Ok(())
    }
}

// ── Loop-guard restorations (issue #4463) ────────────────────────────────────
//
// The TinyAgents migration dropped several loop breakers that the crate does not
// replace (verified against `harness::no_progress`, which tracks *failures*
// only): the recoverable-failure headroom, the terminal delegated-inference
// fast-halt (#3104), the policy-denied fast-trip, and the successful-repeat /
// identical-output guards (#4088 / #4095). These helpers + the
// [`RepeatProgressMiddleware`] below restore that behaviour seam-side, ported
// verbatim from the deleted `agent/harness/tool_loop.rs` thresholds/wording so
// the guards read identically to the legacy loop.

/// Recoverable/transient failures get more identical-retry headroom than the
/// deterministic default: a flaky network call or a timeout can succeed on a
/// later attempt once the model adapts (longer timeout, smaller batch, retry).
/// Mirrors the legacy `RECOVERABLE_REPEAT_FAILURE_THRESHOLD`.
const RECOVERABLE_REPEAT_FAILURE_THRESHOLD: u32 = 8;
/// Recoverable failures also get a larger *consecutive* (varied-args) no-progress
/// headroom before the breaker halts. Mirrors the legacy
/// `RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`.
const RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD: u32 = 12;

/// Clamp the last-error text embedded in a circuit-breaker halt summary so a huge
/// tool error (already capped at 1MB upstream) can't blow up the agent's result.
/// Mirrors the legacy `tool_loop::truncate_for_halt`.
fn truncate_for_halt(s: &str) -> String {
    const MAX: usize = 600;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX).collect();
    format!("{head}\n… [truncated]")
}

/// Failures that are informative and plausibly recoverable by changing the next
/// action (longer timeout, smaller batch, different network retry/fallback)
/// rather than by abandoning the turn. Deliberately marker-based and
/// conservative: it only controls breaker headroom, never converts a failure
/// into success. Ported verbatim from legacy `tool_loop::is_recoverable_tool_failure`.
fn is_recoverable_tool_failure(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "deadline exceeded",
        "temporarily unavailable",
        "temporary failure",
        "connection reset",
        "connection refused",
        "connection closed",
        "connection aborted",
        "network is unreachable",
        "host is unreachable",
        "dns error",
        "failed to lookup address",
        "failed to resolve",
        "rate limit",
        "too many requests",
        "retry after",
        "503 service unavailable",
        "502 bad gateway",
        "504 gateway timeout",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// A permanent, non-retryable inference failure surfaced by a delegated
/// sub-agent's tool result. Unlike a transient error, re-issuing the call cannot
/// succeed even under a *different* delegation tool or varied args: the budget is
/// account-wide and the model/provider configuration is shared by every
/// (sub-)agent. See [`terminal_inference_failure_kind`] (#3104).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TerminalInferenceFailure {
    /// Out of inference budget / credits — every retry hits the same wall.
    BudgetExhausted,
    /// The configured model/provider rejected the request for a reason the user
    /// must fix (unknown model, non-chat/embedding model, missing credential,
    /// region block, …).
    ProviderConfig,
}
