//! [`RepeatedToolFailureMiddleware`]: halt (or nudge) the run when tool calls
//! keep failing with no progress, driving the crate no-progress ladder plus
//! OpenHuman's recoverable-failure headroom and terminal-inference fast-halt.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};
use tinyagents_harness::no_progress::{
    ClassifiedFailure, ClassifiedFailureTracker, NoProgress, NoProgressTracker, ToolAttempt,
};
use tinyagents_harness::steering::{SteeringCommand, SteeringHandle};
use tinyinference_llm::message::Message as TaMessage;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use super::loop_guards::{
    is_recoverable_tool_failure, is_repeat_call_exempt, recoverable_identical_halt_summary,
    recoverable_no_progress_halt_summary, terminal_inference_failure_kind,
    terminal_inference_halt_summary, RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD,
    RECOVERABLE_REPEAT_FAILURE_THRESHOLD,
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
/// - [`NoProgress::Nudge`] — inject the crate's structured "no progress since
///   step X" corrective into the working transcript via
///   [`SteeringCommand::InjectMessage`] so the next model call sees it and
///   changes strategy *before* the same-strategy retry cap trips. (Not
///   `Redirect`: that verb is outside the Interactive steering allowlist and
///   would abort the turn — see the nudge call site.)
/// - [`NoProgress::Halt`] — record the crate's root-cause summary into the shared
///   [`HaltSummarySlot`](crate::agent::tinyagents::HaltSummarySlot) (the turn overrides its final
///   text with it) and pause the run via the shared steering handle (same
///   mechanism as the stop-hook / cap pausers), then [`reset`](NoProgressTracker::reset)
///   so a resumed run does not immediately re-pause on the latched state.
pub(crate) struct RepeatedToolFailureMiddleware {
    handle: SteeringHandle,
    halt_summary: crate::agent::tinyagents::HaltSummarySlot,
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
        halt_summary: crate::agent::tinyagents::HaltSummarySlot,
    ) -> Self {
        Self {
            handle,
            halt_summary,
            tracker: NoProgressTracker::new(identical_threshold),
            classified: ClassifiedFailureTracker::default(),
            step: AtomicUsize::new(0),
            arg_sigs: std::sync::Mutex::new(std::collections::HashMap::new()),
            target_scopes: std::sync::Mutex::new(std::collections::HashMap::new()),
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
        crate::util::truncate_with_ellipsis(error, 400),
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

/// Stable resource identity supplied by the call, excluding free-form queries,
/// prompts, credentials, and URL query parameters. An absent target remains
/// scoped to the operation, never to the changing argument fingerprint.
pub(super) fn failure_scope(tool: &str, arguments: &serde_json::Value) -> String {
    let mut scope = tool.to_owned();
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

/// Explicit recovery policy. Only recognised failures enter the classified
/// ledger; unknown prose continues through the established exact-repeat guard.
pub(super) fn recovery_policy(
    tool: &str,
    error: &str,
    body_level_failure: bool,
) -> Option<(&'static str, usize)> {
    use crate::tools::status::ToolFailureClass as Class;
    if body_level_failure {
        return Some(("validation", 1));
    }
    // An unknown-tool answer is a wrong call the model can correct, and it
    // echoes the attempted name and every valid tool name. Keyword sniffing
    // below would read those names as the failure — `forbidden_tool` or a
    // name carrying `unauthorized` became `authentication`, a zero-retry
    // class, and halted the run on its first wrong guess.
    if error.trim_start().starts_with("unknown tool `") {
        return Some(("validation", 1));
    }
    // A tool-owned JSON error contract is less ambiguous than rendered prose.
    // Read only explicit status/code fields; arbitrary response data is not a
    // failure signal (this function is called only for `is_error` results).
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(error) {
        let status = value
            .get("status_code")
            .or_else(|| value.get("status"))
            .or_else(|| value.pointer("/error/status_code"))
            .and_then(serde_json::Value::as_u64);
        match status {
            Some(401) => return Some(("authentication", 0)),
            Some(403) => return Some(("permission", 0)),
            Some(400 | 422) => return Some(("validation", 1)),
            Some(429 | 500 | 502 | 503 | 504) => return Some(("transient", 2)),
            _ => {}
        }
        let code = value
            .get("code")
            .or_else(|| value.pointer("/error/code"))
            .and_then(serde_json::Value::as_str);
        match code {
            Some("PERMISSION_DENIED") => return Some(("permission", 0)),
            Some("UNAUTHENTICATED") => return Some(("authentication", 0)),
            Some("INVALID_ARGUMENT") => return Some(("validation", 1)),
            Some("WINDOW_NOT_FOUND") => return Some(("missing_window", 1)),
            Some("APP_NOT_FOUND") => return Some(("missing_app", 1)),
            Some("UNIMPLEMENTED") => return Some(("unsupported", 0)),
            Some("UNAVAILABLE" | "RESOURCE_EXHAUSTED") => return Some(("transient", 2)),
            _ => {}
        }
    }
    let class = crate::tools::status::classify(error, false).class;
    Some(match class {
        Class::MissingPermission => ("permission", 0),
        Class::BadCredentials => ("authentication", 0),
        Class::BlockedByPolicy | Class::Denied | Class::ApprovalExpired => ("policy", 0),
        Class::Unsupported | Class::MissingApp => ("unsupported", 0),
        Class::NotFound
            if tool.contains("desktop") && error.to_ascii_lowercase().contains("window") =>
        {
            ("missing_window", 1)
        }
        Class::NotFound => ("not_found", 1),
        Class::ServiceUnavailable | Class::ModelConnection => ("transient", 2),
        Class::Timeout
            if matches!(
                tool,
                "web_search" | "web_fetch" | "file_read" | "list_files" | "desktop_list_windows"
            ) =>
        {
            ("transient", 2)
        }
        Class::Timeout => ("uncertain_side_effect", 0),
        Class::Unknown if is_recoverable_tool_failure(error) => ("transient", 2),
        Class::Unknown
            if error.to_ascii_lowercase().contains("schema validation")
                || error.to_ascii_lowercase().contains("invalid arguments") =>
        {
            ("validation", 1)
        }
        Class::Unknown => return None,
    })
}

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
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
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

        if !result.is_error && !body_level_failure {
            // Only a successful observation against this operation and scope
            // demonstrates that its blocker changed. Unrelated successes do not.
            for class in [
                "permission",
                "authentication",
                "policy",
                "unsupported",
                "missing_window",
                "missing_app",
                "not_found",
                "transient",
                "uncertain_side_effect",
                "validation",
            ] {
                self.classified
                    .clear(&ClassifiedFailure::new(class, tool_name, &scope));
            }
        } else if !is_repeat_call_exempt(tool_name) {
            if let Some((class, budget)) =
                recovery_policy(tool_name, &failure_text, body_level_failure)
            {
                let key = ClassifiedFailure::new(class, tool_name, &scope);
                if let NoProgress::Halt(mut summary) = self.classified.record(&key, budget) {
                    tracing::warn!(
                        tool = tool_name,
                        class,
                        budget,
                        "[tinyagents::mw] classified failure budget exhausted"
                    );
                    if class == "uncertain_side_effect" {
                        summary.push_str(" The action may already have happened; reconcile its external state before any retry.");
                    }
                    if let Ok(mut slot) = self.halt_summary.lock() {
                        *slot = Some(summary);
                    }
                    self.handle.send(SteeringCommand::Pause);
                    self.tracker.reset();
                    return Ok(());
                }
                if matches!(class, "missing_window" | "missing_app" | "validation") {
                    let instruction = if class == "validation" {
                        "The last call failed validation. Correct its schema or arguments once before trying again."
                    } else {
                        "The desktop target was not found. Rediscover the current app and window once before trying again."
                    };
                    self.handle
                        .send(SteeringCommand::InjectMessage(TaMessage::system(
                            instruction,
                        )));
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
        if result.is_error {
            if let Some(kind) = terminal_inference_failure_kind(&failure_text) {
                tracing::warn!(
                    tool = tool_name,
                    kind = ?kind,
                    "[tinyagents::mw] terminal delegated-inference failure — halting on first occurrence with root cause"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(terminal_inference_halt_summary(
                        kind,
                        tool_name,
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
            s.contains(crate::security::POLICY_BLOCKED_MARKER)
                || s.contains(crate::security::POLICY_DENIED_MARKER)
        };
        let hard_reject = policy_marked(&content);

        // ── Part 4: recoverable-failure headroom ────────────────────────────────
        // Transient failures (timeouts, connection resets, rate limits, 5xx) get
        // the legacy extended headroom instead of the crate's deterministic 3/6.
        // Route them to the recoverable ladder; a success or a non-recoverable
        // failure resets that streak and feeds the crate tracker as before.
        let recoverable = result.is_error
            && !hard_reject
            && (is_recoverable_tool_failure(&failure_text)
                || matches!(
                    crate::tools::status::classify(&failure_text, false).class,
                    crate::tools::status::ToolFailureClass::Timeout
                        | crate::tools::status::ToolFailureClass::ServiceUnavailable
                        | crate::tools::status::ToolFailureClass::ModelConnection
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
            if let Some(summary) = self.record_recoverable(tool_name, &arg_fp, &failure_text) {
                tracing::warn!(
                    tool = tool_name,
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
                let escalation = user_actionable_escalation(tool_name, &content);
                let user_actionable = escalation.is_some();
                let summary = escalation.unwrap_or(summary);
                tracing::warn!(
                    tool = tool_name,
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
