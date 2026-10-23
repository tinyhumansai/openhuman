//! [`RepeatedToolFailureMiddleware`]: halt (or nudge) the run when tool calls
//! keep failing with no progress, driving the crate no-progress ladder plus
//! OpenHuman's recoverable-failure headroom and terminal-inference fast-halt.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{
    call_effect, dispatch_target, is_repeat_call_exempt, repeat_guard_marker, FailureScope,
    FailureScopeMiddleware, FailureScopePolicy, FailureStopKind, Middleware, ToolFactsLookup,
    ToolInvocationIdentity,
};
use tinyagents_harness::no_progress::FailurePolicy;
use tinyagents_harness::steering::SteeringHandle;
use tinyagents_hooks::stop::StopLatch;
use tinyagents_hooks::stop::{self, TerminalInferenceFailureKind};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use super::fetched_site::{fetch_host_scope, heuristic_text};
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::util::truncate_with_ellipsis;
use tinyagents_harness::middleware::EphemeralInstructionMiddleware;
use tinyinference_llm::failure::is_recoverable_failure_text as is_recoverable_tool_failure;

pub(crate) fn user_actionable_escalation(tool: &str, error: &str) -> Option<String> {
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
    let missing_connection = lower.contains("[composio:error:composio_platform]")
        || lower.contains("not connected")
        || lower.contains("isn't connected")
        || lower.contains("is not connected")
        || lower.contains("not enabled")
        || lower.contains("token revoked")
        || lower.contains("connection error, try to authenticate");
    missing_connection.then(|| {
        format!(
            "I can't continue without your input: the `{tool}` action needs a service that isn't \
             connected. {}\n\nConnect it (Connections), then tell me to retry — or \
             tell me how you'd like to proceed instead.",
            truncate_with_ellipsis(error, 400),
        )
    })
}

#[cfg(test)]
use tinyagents_harness::no_progress::{
    classified_step, ClassifiedFailureTracker, ClassifiedStep, RecoverableFailureLadder,
};

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
    owner: FailureScopeMiddleware<OpenHumanRunContext>,
    policy: Arc<HostFailurePolicy>,
}

impl RepeatedToolFailureMiddleware {
    /// Build the host policy adapter over TinyAgents' failure-scope driver.
    pub(crate) fn new(
        handle: SteeringHandle,
        identical_threshold: usize,
        halt_summary: tinyagents_harness::turn_outcome::HaltSummarySlot,
    ) -> Self {
        let policy = Arc::new(HostFailurePolicy::new(handle.clone(), halt_summary.clone()));
        let owner = FailureScopeMiddleware::with_identical_threshold(
            handle,
            halt_summary,
            policy.clone(),
            identical_threshold,
        );
        Self { owner, policy }
    }

    /// Return typed classified facts only while this middleware still owns
    /// the halt note recorded for the current run.
    pub(crate) fn current_classified_cause(
        &self,
    ) -> Option<tinyagents_harness::no_progress::ClassifiedStopCause> {
        self.policy.stop.current_classified_cause()
    }

    /// Judge each call's side effect from the registered tools' own
    /// declarations ([`tinyagents_harness::middleware::tool_sets_lookup`]) rather than
    /// from tool names alone.
    pub(crate) fn with_tool_facts(self, lookup: ToolFactsLookup) -> Self {
        *self
            .policy
            .tool_facts
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(lookup);
        self
    }

    /// The request-scoped half of this breaker. Register it **last**: its
    /// `before_model` must run after the transcript snapshot (which a failed
    /// turn persists) and after every reduction step.
    pub(crate) fn nudge_injector(&self) -> EphemeralInstructionMiddleware {
        self.owner.nudge_injector()
    }
}

struct HostFailurePolicy {
    stop: StopLatch,
    tool_facts: Mutex<Option<ToolFactsLookup>>,
    last_exit_report: Mutex<HashMap<u64, String>>,
}

impl HostFailurePolicy {
    fn new(
        handle: SteeringHandle,
        halt_summary: tinyagents_harness::turn_outcome::HaltSummarySlot,
    ) -> Self {
        Self {
            stop: StopLatch::new(handle, halt_summary),
            tool_facts: Mutex::default(),
            last_exit_report: Mutex::default(),
        }
    }
}

/// The halt wording for the recoverable ladder: OpenHuman's own copy.
struct HaltCopy;

impl FailurePolicy for HaltCopy {
    fn identical_halt_summary(&self, tool: &str, count: u32, failure_text: &str) -> String {
        stop::halt_summary::retry_exhausted(
            &crate::agent::tinyagents::host::OpenHumanCopyProvider,
            tool,
            count,
            failure_text,
        )
        .summary
    }

    fn no_progress_halt_summary(&self, consecutive: u32, tool: &str, failure_text: &str) -> String {
        stop::halt_summary::no_progress(
            &crate::agent::tinyagents::host::OpenHumanCopyProvider,
            consecutive,
            tool,
            failure_text,
        )
        .summary
    }
}

/// The first line of a failure (see the harness `first_error_line`), scrubbed
/// first: the line is read by the model and persisted with the session in a
/// halt summary, and a command can print a token or a user's own words on stderr.
fn first_error_line(text: &str) -> String {
    tinyagents_harness::no_progress::first_error_line(&sanitized_failure(text))
}

fn sanitized_failure(text: &str) -> String {
    crate::security::redact::text(text)
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

impl FailurePolicy for HostFailurePolicy {
    fn identical_halt_summary(&self, tool: &str, count: u32, failure_text: &str) -> String {
        HaltCopy.identical_halt_summary(tool, count, failure_text)
    }

    fn no_progress_halt_summary(&self, consecutive: u32, tool: &str, failure_text: &str) -> String {
        HaltCopy.no_progress_halt_summary(consecutive, tool, failure_text)
    }
}

impl FailureScopePolicy<OpenHumanRunContext> for HostFailurePolicy {
    fn scope(&self, _ctx: &RunContext<OpenHumanRunContext>, call: &TaToolCall) -> FailureScope {
        FailureScope {
            argument_fingerprint: args_fingerprint(&call.arguments),
            scope: failure_scope(&call.name, &call.arguments),
        }
    }

    fn assess(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _invocation: &ToolInvocationIdentity,
        call: &TaToolCall,
        _scope: &FailureScope,
        result: &TaToolResult,
    ) -> tinyagents_harness::middleware::FailureAssessment {
        use tinyagents_harness::middleware::FailureAssessment;

        if crate::memory::tools::is_budget_refusal(result) {
            return FailureAssessment::Ignore;
        }
        let content = crate::agent::tinyagents::middleware::tool_result_text(result);
        let body_failure = !result.is_error && is_body_level_failure(&call.name, &content);
        let failed = result.is_error || body_failure;
        if !failed {
            return FailureAssessment::Progress;
        }

        let failure_text = sanitized_failure(&content);
        let heuristic = heuristic_text(&call.name, &failure_text);
        if result.is_error && !is_command_exit_report(&failure_text) {
            if let Some(kind) = stop::terminal_inference_failure_kind(heuristic) {
                let halt = match kind {
                    TerminalInferenceFailureKind::BudgetExhausted => stop::halt_summary::budget(
                        &crate::agent::tinyagents::host::OpenHumanCopyProvider,
                        &call.name,
                        &failure_text,
                    ),
                    TerminalInferenceFailureKind::ProviderConfig => {
                        stop::halt_summary::provider_config(
                            &crate::agent::tinyagents::host::OpenHumanCopyProvider,
                            &call.name,
                            &failure_text,
                        )
                    }
                };
                return FailureAssessment::Halt {
                    summary: halt.summary.clone(),
                    cause: Some(tinyagents_harness::no_progress::ClassifiedStopCause {
                        class: halt.class.as_str().to_owned(),
                        operation: call.name.clone(),
                        summary: halt.summary,
                    }),
                };
            }
        }

        let hard_reject = content.contains(tinytools::status::POLICY_BLOCKED_MARKER)
            || content.contains(tinytools::status::POLICY_DENIED_MARKER);
        if !is_repeat_call_exempt(&call.name) {
            let effect = call_effect(
                self.tool_facts
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref(),
                &call.name,
                &call.arguments,
            );
            if let Some((class, budget)) =
                recovery_policy_with_effect(&call.name, &failure_text, body_failure, effect)
            {
                let nudge = (class == "missing_app" && call.name == "shell")
                    .then(|| missing_program_nudge(&call.name, &first_error_line(&failure_text)));
                return FailureAssessment::ClassifiedFailure {
                    class: class.to_owned(),
                    budget,
                    error: failure_text,
                    nudge,
                };
            }
        }

        let recoverable = result.is_error
            && !hard_reject
            && !is_command_exit_report(&failure_text)
            && (is_recoverable_tool_failure(heuristic)
                || matches!(
                    tinytools::status::classify(heuristic, false).class,
                    tinytools::status::ToolFailureClass::Timeout
                        | tinytools::status::ToolFailureClass::ServiceUnavailable
                        | tinytools::status::ToolFailureClass::ModelConnection
                ));
        if recoverable {
            if is_repeat_call_exempt(&call.name) {
                return FailureAssessment::Ignore;
            }
            return FailureAssessment::RecoverableFailure {
                error: failure_text,
            };
        }

        FailureAssessment::Failure {
            error: failure_text,
            hard_reject,
            recoverable_miss: false,
        }
    }

    fn halt_copy_for_call(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        kind: FailureStopKind,
        summary: String,
        error: &str,
        call: &TaToolCall,
    ) -> String {
        if kind == FailureStopKind::NoProgress && !is_command_exit_report(error) {
            if let Some(escalation) = user_actionable_escalation(&call.name, error) {
                return escalation;
            }
        }
        summary
    }

    fn halt_cause_for_call(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        kind: FailureStopKind,
        cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
        error: &str,
        call: &TaToolCall,
    ) -> Option<tinyagents_harness::no_progress::ClassifiedStopCause> {
        if kind == FailureStopKind::NoProgress
            && !is_command_exit_report(error)
            && user_actionable_escalation(&call.name, error).is_some()
        {
            return Some(tinyagents_harness::no_progress::ClassifiedStopCause {
                class: "missing_connection".into(),
                operation: call.name.clone(),
                summary: String::new(),
            });
        }
        cause
    }

    fn start_batch(&self, _ctx: &RunContext<OpenHumanRunContext>) {
        self.stop.start_batch();
    }

    fn is_repeat_guard_refusal(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
        result: &TaToolResult,
    ) -> bool {
        repeat_guard_marker(result).is_some()
    }

    fn record_guard_refusal(&self, _ctx: &RunContext<OpenHumanRunContext>) {
        self.stop.record_refusal();
    }

    fn record_progress(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
        _scope: &FailureScope,
    ) {
        self.stop.record_success();
        self.last_exit_report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&ctx.instance_id());
    }

    fn current_halt_cause(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
    ) -> Option<tinyagents_harness::no_progress::ClassifiedStopCause> {
        self.stop.current_classified_cause()
    }

    fn reset_no_progress_before_failure(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
        scope: &FailureScope,
        result: &TaToolResult,
    ) -> bool {
        let content = crate::agent::tinyagents::middleware::tool_result_text(result);
        if !result.is_error || !is_command_exit_report(&content) {
            self.last_exit_report
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&ctx.instance_id());
            return false;
        }
        let key = format!("{}\u{1f}{}", call.name, scope.argument_fingerprint);
        let mut reports = self
            .last_exit_report
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let is_new_command = reports.get(&ctx.instance_id()) != Some(&key);
        reports.insert(ctx.instance_id(), key);
        is_new_command
    }

    fn record_failure(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
        scope: &FailureScope,
        _result: &TaToolResult,
        error: &str,
    ) -> bool {
        self.stop
            .witness()
            .record_failure(&call.name, &scope.argument_fingerprint, error)
    }

    fn classified_halt(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        summary: &str,
        class: &str,
        call: &TaToolCall,
        repeated: bool,
        error: &str,
    ) {
        self.stop
            .record_classified_halt(summary.to_owned(), class, &call.name, repeated, || {
                first_error_line(error)
            });
    }

    fn finish_run(&self, ctx: &RunContext<OpenHumanRunContext>) {
        self.stop.finish_run();
        self.last_exit_report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&ctx.instance_id());
    }
}

#[async_trait]
impl Middleware<(), OpenHumanRunContext> for RepeatedToolFailureMiddleware {
    fn name(&self) -> &str {
        "repeated_tool_failure"
    }

    fn is_observer(&self) -> bool {
        true
    }

    async fn after_model(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        state: &(),
        response: &mut tinyinference_llm::model::ModelResponse,
    ) -> TaResult<()> {
        self.owner.after_model(ctx, state, response).await
    }

    async fn before_tool(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        self.owner.before_tool(ctx, state, call).await
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        self.owner.after_tool(ctx, state, invocation, result).await
    }

    async fn after_agent(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        state: &(),
        run: &mut tinyagents_harness::middleware::AgentRun,
    ) -> TaResult<()> {
        // The owner's `after_agent` finishes the run and clears the stop latch,
        // so read the typed cause first or it is lost on a classified halt.
        if run.failure_cause.is_none() {
            run.failure_cause = self.policy.stop.current_classified_cause();
        }
        self.owner.after_agent(ctx, state, run).await
    }

    async fn on_error(
        &self,
        ctx: &mut RunContext<OpenHumanRunContext>,
        error: &tinyagents_harness::error::TinyAgentsError,
    ) -> TaResult<()> {
        self.owner.on_error(ctx, error).await
    }
}

#[cfg(test)]
#[path = "repeated_failure_tests.rs"]
mod tests;
