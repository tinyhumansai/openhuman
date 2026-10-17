//! [`RepeatedToolFailureMiddleware`]: halt (or nudge) the run when tool calls
//! keep failing with no progress, driving the crate no-progress ladder plus
//! OpenHuman's recoverable-failure headroom and terminal-inference fast-halt.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{
    call_effect, dispatch_target, is_repeat_call_exempt, repeat_guard_marker, CallEffect,
    Middleware, StopLatch, ToolFactsLookup, ToolInvocationIdentity,
};
use tinyagents_harness::no_progress::{
    classified_step, clear_recovered, ClassifiedFailureTracker, ClassifiedStep, FailurePolicy,
    NoProgress, NoProgressTracker, RecoverableFailureLadder, ToolAttempt,
};
use tinyagents_harness::steering::SteeringHandle;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use super::fetched_site::{fetch_host_scope, heuristic_text};
pub(crate) use crate::inference::failure_copy::user_actionable_escalation;
use crate::inference::failure_copy::{
    recoverable_identical_halt_summary, recoverable_no_progress_halt_summary,
    terminal_inference_failure_kind, terminal_inference_halt_summary,
};
use tinyagents_harness::middleware::{EphemeralInstructionMiddleware, EphemeralInstructionQueue};
use tinyinference_llm::failure::is_recoverable_failure_text as is_recoverable_tool_failure;

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
/// - [`NoProgress::Nudge`] — queue the crate's structured "no progress since
///   step X" corrective for the **next model request only** (see
///   [`PendingNudgeInjector`]) so the model changes strategy *before* the
///   same-strategy retry cap trips.
/// - [`NoProgress::Halt`] — record the crate's root-cause summary into the shared
///   [`HaltSummarySlot`](tinyagents_harness::turn_outcome::HaltSummarySlot) (the turn overrides its final
///   text with it) and pause the run via the shared steering handle (same
///   mechanism as the stop-hook / cap pausers), then [`reset`](NoProgressTracker::reset)
///   so a resumed run does not immediately re-pause on the latched state.
pub(crate) struct RepeatedToolFailureMiddleware {
    /// Pauses the run and records why in the shared stop-note slot.
    stop: StopLatch,
    /// Crate no-progress escalation ladder — the single source of the
    /// identical-failure / varied-failure / hard-reject logic (tinyagents 1.5.0).
    tracker: NoProgressTracker,
    classified: ClassifiedFailureTracker,
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
    /// Call ID to stable target identity. Query strings and free-form prompts
    /// are excluded so varying a query cannot evade a resource-level blocker.
    target_scopes: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Recoverable-failure ladder (issue #4463): transient failures (timeouts,
    /// connection resets, rate limits, 5xx) are routed here instead of the crate
    /// tracker so they get the legacy extended headroom
    /// ([`RECOVERABLE_REPEAT_FAILURE_THRESHOLD`] identical /
    /// [`RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`] consecutive) rather than the
    /// crate's fixed 3/6, which is right only for deterministic failures.
    /// Counting lives in the harness ladder; the halt wording is
    /// [`HaltCopy`].
    recoverable: RecoverableFailureLadder,
    /// Corrective nudges for the next model request, drained by
    /// [`PendingNudgeInjector`]. Never sent through steering: an injected
    /// message joins the working transcript and is committed into durable
    /// history, where it replays as a stale instruction on every later turn
    /// (#6725).
    pending_nudges: EphemeralInstructionQueue,
    /// `tool\u{1f}args` of the last finished command that exited non-zero,
    /// kept so a *repeat* of it still counts as a failure while a different
    /// command's non-zero exit counts as information (see `after_tool`).
    last_exit_report: std::sync::Mutex<Option<String>>,
    /// Call ID to what the call may have done ([`call_effect`]), judged in
    /// `before_tool` where the arguments are visible. Decides whether a
    /// timeout is a retryable read or an uncertain action.
    call_effects: std::sync::Mutex<std::collections::HashMap<String, CallEffect>>,
    /// The registered tools' declarations (read-only policy, external effect,
    /// permission level). Without it only tool names are read.
    tool_facts: Option<ToolFactsLookup>,
}

impl RepeatedToolFailureMiddleware {
    /// Build the breaker. `identical_threshold` (the identical-signature retry
    /// ceiling) is handed straight to [`NoProgressTracker::new`], which clamps it
    /// so a nudge always precedes a halt (a single failure is never a loop).
    pub(crate) fn new(
        handle: SteeringHandle,
        identical_threshold: usize,
        halt_summary: tinyagents_harness::turn_outcome::HaltSummarySlot,
    ) -> Self {
        Self {
            stop: StopLatch::new(handle, halt_summary),
            tracker: NoProgressTracker::new(identical_threshold),
            classified: ClassifiedFailureTracker::default(),
            last_exit_report: std::sync::Mutex::default(),
            step: AtomicUsize::new(0),
            arg_sigs: std::sync::Mutex::new(std::collections::HashMap::new()),
            target_scopes: std::sync::Mutex::new(std::collections::HashMap::new()),
            recoverable: RecoverableFailureLadder::default(),
            pending_nudges: EphemeralInstructionQueue::new(),
            call_effects: std::sync::Mutex::new(std::collections::HashMap::new()),
            tool_facts: None,
        }
    }

    /// Judge each call's side effect from the registered tools' own
    /// declarations ([`tinyagents_harness::middleware::tool_sets_lookup`]) rather than
    /// from tool names alone.
    pub(crate) fn with_tool_facts(mut self, lookup: ToolFactsLookup) -> Self {
        self.tool_facts = Some(lookup);
        self
    }

    /// The request-scoped half of this breaker. Register it **last**: its
    /// `before_model` must run after the transcript snapshot (which a failed
    /// turn persists) and after every reduction step.
    pub(crate) fn nudge_injector(&self) -> EphemeralInstructionMiddleware {
        self.pending_nudges.middleware()
    }

    /// Inspect pending product corrections in tests.
    #[cfg(test)]
    pub(crate) fn take_pending_nudges(&self) -> Vec<String> {
        self.pending_nudges.drain_all()
    }
}

/// The halt wording for the recoverable ladder: OpenHuman's own copy.
struct HaltCopy;

impl FailurePolicy for HaltCopy {
    fn identical_halt_summary(&self, tool: &str, count: u32, failure_text: &str) -> String {
        recoverable_identical_halt_summary(tool, count, failure_text)
    }

    fn no_progress_halt_summary(&self, consecutive: u32, tool: &str, failure_text: &str) -> String {
        recoverable_no_progress_halt_summary(consecutive, tool, failure_text)
    }
}

/// The first line of a failure (see the harness `first_error_line`), scrubbed
/// first: the line is read by the model and persisted with the session in a
/// halt summary, and a command can print a token or a user's own words on stderr.
fn first_error_line(text: &str) -> String {
    tinyagents_harness::no_progress::first_error_line(&sanitized_failure(text))
}

fn sanitized_failure(text: &str) -> String {
    crate::security::scrub::sanitize_text(text).value
}

/// A stable, bounded fingerprint of a tool call's arguments for the identical-
/// repeat signature (hashed so a huge payload doesn't bloat the map/comparison).
fn args_fingerprint(arguments: &serde_json::Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    arguments.to_string().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Stable resource identity supplied by the call, excluding free-form queries,
/// prompts, credentials, and URL query parameters. An absent target remains
/// scoped to the operation, never to the changing argument fingerprint.
///
/// A dispatcher (`use_skill`, `composio_execute`) is scoped by the skill and
/// tool it reaches, and by that inner call's resource fields: every
/// `use_skill` failure used to share one budget, so a timeout in one skill's
/// sub-tool and a refusal in another's halted the turn together.
pub(super) fn failure_scope(tool: &str, arguments: &serde_json::Value) -> String {
    let mut scope = tool.to_owned();
    let mut resource_args = arguments;
    if let Some(target) = dispatch_target(tool, arguments) {
        if let Some(skill) = target.skill {
            scope.push_str(":skill=");
            scope.push_str(&crate::util::truncate_with_ellipsis(skill, 80));
        }
        scope.push_str(":tool=");
        scope.push_str(&crate::util::truncate_with_ellipsis(target.tool, 120));
        if let Some(inner) = target.args.filter(|a| a.is_object()) {
            resource_args = inner;
        }
    }
    let arguments = resource_args;
    for field in [
        "account_id",
        "workspace_id",
        "app",
        "window_id",
        "resource",
        "endpoint",
        "url",
    ] {
        let value = match arguments.get(field) {
            Some(serde_json::Value::String(value)) if !value.is_empty() => value.clone(),
            Some(serde_json::Value::Number(value)) => value.to_string(),
            _ => continue,
        };
        if let Some(host_scope) = fetch_host_scope(tool, field, &value) {
            scope.push_str(&host_scope);
            continue;
        }
        scope.push(':');
        scope.push_str(field);
        scope.push('=');
        scope.push_str(&crate::util::truncate_with_ellipsis(
            value.split('?').next().unwrap_or(&value),
            120,
        ));
    }
    scope
}

#[cfg(test)]
pub(super) use super::failure_policy::recovery_policy;
pub(super) use super::failure_policy::{missing_program_nudge, recovery_policy_with_effect};
use tinyagents_harness::no_progress::is_command_exit_report;

/// Detect a **body-level** failure from `validate_workflow` / `dry_run_workflow`
/// (issue: flows breaker doesn't see repeated invalid-graph loops). Both tools
/// report an invalid graph / aborted sandbox run via `ToolResult::success` with
/// a JSON body carrying top-level `"ok": false`
/// (`crates/openhuman-core/src/flows/builder_tools.rs`) rather than `ToolResult::error` — so
/// `result.error` stays `None` and the no-progress breaker below never counts
/// the repeat as a failure, letting a graph the model can't fix burn the whole
/// iteration budget instead of tripping the same nudge/halt ladder.
///
/// Scoped to exactly these two tool names: a generic `"ok": false` in some other
/// tool's JSON body may be legitimate data (not a failure signal), so this must
/// not reinterpret arbitrary tool output. Tolerant of non-JSON or missing `ok`
/// content — returns `false` rather than guessing.
pub(crate) fn is_body_level_failure(name: &str, content: &str) -> bool {
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
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for RepeatedToolFailureMiddleware
{
    fn name(&self) -> &str {
        "repeated_tool_failure"
    }

    // Failure accounting and corrective nudges must observe tool outcomes even
    // when an earlier middleware has already requested a control action.
    fn is_observer(&self) -> bool {
        true
    }

    /// A new model response starts a new tool batch for the stop latch.
    async fn after_model(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        _response: &mut tinyinference_llm::model::ModelResponse,
    ) -> TaResult<()> {
        self.stop.start_batch();
        Ok(())
    }

    /// Forget the run's stop-latch state (see [`StopLatch::finish_run`]).
    async fn after_agent(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        _run: &mut tinyagents_harness::middleware::AgentRun,
    ) -> TaResult<()> {
        self.stop.finish_run();
        Ok(())
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // The tool result carries no arguments, so capture a fingerprint here and
        // correlate it by call_id in `after_tool`.
        if let Ok(mut sigs) = self.arg_sigs.lock() {
            sigs.insert(call.id.clone(), args_fingerprint(&call.arguments));
        }
        if let Ok(mut scopes) = self.target_scopes.lock() {
            scopes.insert(call.id.clone(), failure_scope(&call.name, &call.arguments));
        }
        let effect = call_effect(self.tool_facts.as_ref(), &call.name, &call.arguments);
        tracing::trace!(
            tool = %call.name,
            call_id = %call.id,
            effect = ?effect,
            "[tinyagents::mw] tool call side effect judged for failure classification"
        );
        if let Ok(mut effects) = self.call_effects.lock() {
            effects.insert(call.id.clone(), effect);
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let tool_name = invocation.tool_name();
        let content = crate::agent::tinyagents::middleware::tool_result_text(result);
        let arg_fp = self
            .arg_sigs
            .lock()
            .ok()
            .and_then(|mut sigs| sigs.remove(&invocation.call_id().to_string()))
            .unwrap_or_default();
        let scope = self
            .target_scopes
            .lock()
            .ok()
            .and_then(|mut scopes| scopes.remove(&invocation.call_id().to_string()))
            .unwrap_or_else(|| tool_name.to_owned());
        let effect = self
            .call_effects
            .lock()
            .ok()
            .and_then(|mut effects| effects.remove(&invocation.call_id().to_string()))
            .unwrap_or_default();
        // A result the repeat guard answered itself (blocked/halted without
        // running the tool) is the guard's verdict, not the tool failing, so it
        // must not feed the failure ladder.
        if let Some(marker) = repeat_guard_marker(result) {
            tracing::debug!(
                tool = tool_name,
                marker,
                "[tinyagents::mw] skipping repeat-guard result in failure ladder"
            );
            self.stop.record_refusal();
            return Ok(());
        }
        // The memory budget refusing a call past its cap is the host's own
        // limit (`memory::tool_budget`), not a failure: a parallel batch of
        // memory calls past the cap must not halt the run (#7717).
        if crate::memory::tools::is_budget_refusal(result) {
            tracing::debug!(
                tool = tool_name,
                "[tinyagents::mw] skipping memory budget refusal in failure ladder"
            );
            return Ok(());
        }
        let step = self.step.fetch_add(1, Ordering::SeqCst) + 1;

        // Body-level failure signal: `validate_workflow` / `dry_run_workflow`
        // report an invalid graph via a `success` result whose JSON body carries
        // `"ok": false` — see `is_body_level_failure`. Only meaningful when
        // `result.error` is `None`; when both are set, `result.error` already
        // drives every check below, so this never double-counts one failure.
        let body_level_failure = !result.is_error && is_body_level_failure(tool_name, &content);

        // Combined failure text for classification: the model-facing content plus
        // the (redundant but authoritative) error field. Both are scanned for the
        // policy / terminal-inference / recoverable markers below.
        let failure_text = match result.is_error {
            true => content.clone(),
            false if body_level_failure => content.clone(),
            false => String::new(),
        };
        let heuristic_failure_text = heuristic_text(tool_name, &failure_text);
        let failed = result.is_error || body_level_failure;
        let repeated_failure = failed
            && self
                .stop
                .witness()
                .record_failure(tool_name, &arg_fp, &failure_text);

        if !failed {
            self.stop.record_success();
            // Only a successful observation against this operation and scope
            // demonstrates that its blocker changed. Unrelated successes do not.
            clear_recovered(&self.classified, tool_name, &scope);
        } else if !is_repeat_call_exempt(tool_name) {
            if let Some((class, budget)) =
                recovery_policy_with_effect(tool_name, &failure_text, body_level_failure, effect)
            {
                match classified_step(
                    &self.classified,
                    class,
                    tool_name,
                    &scope,
                    budget,
                    &sanitized_failure(&failure_text),
                ) {
                    ClassifiedStep::Halt(summary) => {
                        tracing::warn!(
                            tool = tool_name,
                            class,
                            budget,
                            "[tinyagents::mw] classified failure budget exhausted"
                        );
                        // An identical re-issue made while the repeat guard
                        // refuses the calls around it is a repeat loop.
                        self.stop.halt_classified(
                            summary,
                            class,
                            tool_name,
                            repeated_failure,
                            || first_error_line(&failure_text),
                        );
                        self.tracker.reset();
                        return Ok(());
                    }
                    ClassifiedStep::Nudge(generic) => {
                        // The harness wording for `missing_app` is the
                        // desktop-target one; a shell command names this
                        // host's OS and shell instead.
                        let instruction = if class == "missing_app" && tool_name == "shell" {
                            missing_program_nudge(tool_name, &first_error_line(&failure_text))
                        } else {
                            generic
                        };
                        tracing::debug!(
                            tool = tool_name,
                            class,
                            "[tinyagents::mw] classified failure within budget — nudging recovery"
                        );
                        self.pending_nudges.push(ctx, instruction);
                    }
                    ClassifiedStep::Within => {}
                }
                // The classified budget owns this known blocker. In particular,
                // a different query must not reset its count or trigger a
                // competing exact-repeat nudge.
                return Ok(());
            }
        }

        // ── Part 5 (#3104): terminal delegated-inference fast-halt ──────────────
        // A permanent inference failure (out of budget / provider-config rejection)
        // surfaced by a delegated sub-agent cannot be recovered by retrying — the
        // budget is account-wide and the model/provider config is shared by every
        // (sub-)agent. Halt on the FIRST occurrence with an actionable root cause,
        // *before* the count-based thresholds, because the orchestrator otherwise
        // re-emits the doomed step under varied delegation-tool names so the
        // identical-retry threshold never trips in time.
        // A command's exit report carries the program's output, which can quote
        // a provider error (a script calling an API) without the agent's own
        // inference having failed.
        if result.is_error && !is_command_exit_report(&failure_text) {
            if let Some(kind) = terminal_inference_failure_kind(heuristic_failure_text) {
                tracing::warn!(
                    tool = tool_name,
                    kind = ?kind,
                    "[tinyagents::mw] terminal delegated-inference failure — halting on first occurrence with root cause"
                );
                self.stop.halt(terminal_inference_halt_summary(
                    kind,
                    tool_name,
                    &failure_text,
                ));
                self.tracker.reset();
                self.recoverable.reset_streak();
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
            s.contains(tinytools::status::POLICY_BLOCKED_MARKER)
                || s.contains(tinytools::status::POLICY_DENIED_MARKER)
        };
        let hard_reject = policy_marked(&content);

        // ── Part 4: recoverable-failure headroom ────────────────────────────────
        // Transient failures (timeouts, connection resets, rate limits, 5xx) get
        // the legacy extended headroom instead of the crate's deterministic 3/6.
        // Route them to the recoverable ladder; a success or a non-recoverable
        // failure resets that streak and feeds the crate tracker as before.
        // A finished command's exit report is the program's output, so a test
        // run that prints `timed out` or `connection refused` is not a
        // transient tool failure. Its identical-repeat count would otherwise
        // persist across the turn and halt an edit-and-rerun loop on the same
        // test command; the crate tracker below resets on any success instead.
        let recoverable = result.is_error
            && !hard_reject
            && !is_command_exit_report(&failure_text)
            && (is_recoverable_tool_failure(heuristic_failure_text)
                || matches!(
                    tinytools::status::classify(heuristic_failure_text, false).class,
                    tinytools::status::ToolFailureClass::Timeout
                        | tinytools::status::ToolFailureClass::ServiceUnavailable
                        | tinytools::status::ToolFailureClass::ModelConnection
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
            if is_repeat_call_exempt(tool_name) {
                return Ok(());
            }
            if let Some(summary) =
                self.recoverable
                    .record(&HaltCopy, tool_name, &arg_fp, &failure_text)
            {
                tracing::warn!(
                    tool = tool_name,
                    "[tinyagents::mw] recoverable-failure headroom exhausted — halting run so the root cause surfaces"
                );
                self.stop.halt(summary);
                self.recoverable.reset_streak();
            }
            // Recoverable failures never feed the crate tracker — its fixed 3/6
            // backstop would halt them before the extended headroom is spent.
            return Ok(());
        }
        // Success or non-recoverable failure: clear the recoverable streak (its
        // per-signature counts persist across the turn) before the crate tracker
        // handles the deterministic 3/6 + hard-reject-2 path below.
        self.recoverable.reset_streak();

        // Union the body-level `ok:false` signal with the existing `error.is_some()`
        // predicate so the crate tracker (which reads `attempt.error` as its sole
        // success/failure signal — `None` means "progress was made, reset every
        // counter") sees the repeat as a failure and feeds it into the same
        // nudge/halt ladder as a real tool error.
        // A finished command that exited non-zero is the program's answer, and
        // a different command's non-zero exit is new information, not a
        // repeat: `pip install` failing to build, `g++` turning out to be
        // missing, a `which cc` that finds nothing. Six such answers in a row
        // ended one turn 39 s into a 60-minute budget as "no progress". Only
        // the same command failing again counts toward the ladder; a different
        // one resets it the way a success would. A loop of varied commands is
        // still bounded by the call cap and the clock.
        let exit_report = result.is_error && !hard_reject && is_command_exit_report(&failure_text);
        let same_command_again = exit_report && {
            let key = format!("{tool_name}\u{1f}{arg_fp}");
            let mut last = self.last_exit_report.lock().ok();
            let repeat = last
                .as_deref()
                .is_some_and(|l| l.as_deref() == Some(key.as_str()));
            if let Some(slot) = last.as_mut() {
                **slot = Some(key);
            }
            repeat
        };
        if !exit_report {
            if let Ok(mut last) = self.last_exit_report.lock() {
                *last = None;
            }
        }
        if exit_report && !same_command_again {
            // A new command: the ladder starts over, as after a success, and
            // the list a halt would print starts with this call.
            self.tracker.reset();
        }
        let attempt_error: Option<&str> = match result.is_error {
            true => Some(failure_text.as_str()),
            false if body_level_failure => Some(failure_text.as_str()),
            false => None,
        };
        let attempt = ToolAttempt {
            tool: tool_name,
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
                    tool = tool_name,
                    step,
                    hard_reject,
                    "[tinyagents::mw] no-progress nudge — steering the model to change strategy before the retry cap"
                );
                // Request-scoped, not steering: no run policy can reject it (a
                // `Redirect` nudge aborted interactive turns, #4473), and it is
                // never committed into durable history (#6725).
                self.pending_nudges.push(ctx, instruction);
            }
            NoProgress::Halt(summary) => {
                // #4092: if the blocker is user-actionable (a missing connection),
                // escalate with a concrete ask instead of the crate's generic
                // "unreachable environment, report back" summary.
                // A command printing `not connected` is not a missing integration.
                let escalation = (!is_command_exit_report(&content))
                    .then(|| user_actionable_escalation(tool_name, &content))
                    .flatten();
                let user_actionable = escalation.is_some();
                let summary = escalation.unwrap_or(summary);
                tracing::warn!(
                    tool = tool_name,
                    step,
                    hard_reject,
                    user_actionable,
                    "[tinyagents::mw] repeated tool failure — halting run so the root cause surfaces"
                );
                // Pause at the top of the next iteration (before the next model
                // call), matching the stop-hook / cap pause path. Reset so a
                // resumed run does not immediately re-pause on the latched state
                // (the crate also resets internally on a halt; this is explicit
                // and idempotent).
                self.stop.halt(summary);
                self.tracker.reset();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "repeated_failure_tests.rs"]
mod tests;
