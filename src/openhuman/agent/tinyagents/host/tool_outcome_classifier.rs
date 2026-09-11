//! Host implementation of [`ToolOutcomeClassifier`] over OpenHuman's
//! `tool_status` domain.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The agent runtime knows a tool
//! call returned; it does not know whether the thing that came back is a
//! success, something to re-dispatch, or a dead end. OpenHuman already owns that
//! judgement in [`crate::openhuman::tool_status`], whose
//! [`classify`](crate::openhuman::tools::status::classify) turns raw tool error
//! text into a [`ClassifiedFailure`](crate::openhuman::tools::status::ClassifiedFailure)
//! (a [`ToolFailureClass`] plus a user-facing category). This adapter is the one
//! place that mapping is projected onto the crate's three-way
//! [`OutcomeClass`].
//!
//! # Contract mismatches resolved here
//!
//! **1. `ClassifiedFailure::recoverable` is NOT the retryability signal.**
//! The tempting one-liner is `if failure.recoverable { RetryableFailure }`. It
//! is wrong. `recoverable` means `category == FailureCategory::Recoverable`, and
//! [`ToolFailureClass::Unknown`] — *anything the heuristic could not classify* —
//! sits in that category so the UI can offer "try again" copy. The crate's
//! contract for [`OutcomeClass::RetryableFailure`] is much stronger: it is "an
//! assertion by the host that repeating is acceptable", made without knowing
//! whether the tool had side effects. An unclassified failure is precisely the
//! case where OpenHuman does *not* know that. So this adapter branches on the
//! **class**, not on `recoverable`, and only the three transient classes are
//! called retryable.
//!
//! That is not a new policy invention — it is the split OpenHuman's own steering
//! middleware already makes. `TinyAgents`' recoverable-failure headroom ladder
//! (`middleware.rs`, issue #4463 part 4) gates on exactly
//! `Timeout | ServiceUnavailable | ModelConnection` and lets everything else,
//! `Unknown` included, fall through to the hard-failure path. Mapping `Unknown`
//! to [`OutcomeClass::PermanentFailure`] keeps this seam consistent with that
//! guard and matches the crate's own documented safe default
//! (`ErrorFieldClassifier` classifies every error as permanent for the same
//! side-effect reason).
//!
//! **2. `classify` wants a `timed_out` flag the runtime does not carry.**
//! [`ToolResult`] has no "the executor stopped this at its deadline" bit, so the
//! flag is derived the same way `TinyAgentsToolStatusMiddleware` derives it —
//! sniffing `"timed out"` out of the combined failure text. Doing it identically
//! is the point: the middleware and this classifier must not disagree about
//! whether a call timed out.
//!
//! **3. Error text lives in two places.** The middleware combines
//! [`ToolResult::error`] and [`ToolResult::content`] before classifying (#4459),
//! because the policy markers and timeout phrases are emitted by the tool layer
//! into whichever of the two it had to hand. This adapter uses the same
//! combination, so a `[policy-denied]` marker is honoured wherever it landed and
//! a user refusal can never be re-dispatched.
//!
//! **4. A timeout cannot be assumed safe to repeat.** `Timeout` is the one
//! failure class where "the call failed" and "the call succeeded but the reply
//! was lost" are indistinguishable from the outside. For a tool with an
//! external effect — sending an email, moving money, running a shell command —
//! re-dispatching a timed-out call can commit the effect a second time. The
//! crate's `RetryableFailure` is an assertion that repeating is *acceptable*,
//! so it may only be made about a tool the host has positively identified as
//! side-effect free — declared through
//! [`OpenHumanToolOutcomeClassifier::with_retry_safe_tools`] as an
//! **allowlist**. An allowlist rather than the inverse, because
//! `Tool::external_effect()` is arg-less: `ShellTool` and other raw
//! tool classify their effect from *arguments* (`external_effect_with_args`)
//! and leave the arg-less variant at the default `false`, so a denylist built
//! from it would admit precisely the tools that must be excluded. Absent the
//! allowlist, timeouts stay permanent — the safe direction, since a lost retry
//! costs an iteration while a duplicated payment cannot be undone.
//!
//! The adapter is pure, as the trait requires: no I/O, no interior mutability,
//! same answer for the same `(name, result)` every time.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_harness::host::{OutcomeClass, ToolOutcomeClassifier};
use tinyagents_harness::tool::ToolResult;

use crate::openhuman::tools::status::{classify, ToolFailureClass};

/// OpenHuman's [`ToolOutcomeClassifier`], backed by
/// [`crate::openhuman::tools::status::classify`].
///
/// Zero-sized and stateless: the classifier's whole knowledge base is the
/// keyword heuristic in the `tool_status` domain, which is itself pure. Every
/// instance behaves identically, so callers may construct one per session
/// without cost.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenHumanToolOutcomeClassifier {
    /// Names of tools the host positively knows are safe to call twice.
    ///
    /// An **allowlist**, not a denylist, and deliberately so. The obvious
    /// inverse — "the tools that declare `Tool::external_effect()`" — is unsafe
    /// here, because a tool whose effect depends on its arguments overrides
    /// `external_effect_with_args` and leaves the arg-less
    /// `external_effect()` at the trait default of `false`. `ShellTool` is
    /// exactly that shape, so a denylist built from the arg-less signal would
    /// have called a timed-out shell write retryable. Anything not named here
    /// is treated as potentially effectful.
    ///
    /// Only consulted for [`ToolFailureClass::Timeout`], where "the call failed"
    /// and "the reply was lost after the effect committed" are
    /// indistinguishable from the outside. See [`Self::class_of`].
    retry_safe_tools: Option<Arc<HashSet<String>>>,
}

impl OpenHumanToolOutcomeClassifier {
    /// Creates the classifier with no retry-safe tools declared.
    ///
    /// Timeouts are then treated as **permanent**, because a classifier that
    /// cannot tell an email sender from a file read must not promise that
    /// repeating the call is safe. Attach [`Self::with_retry_safe_tools`] to
    /// recover retries for the tools that can afford them.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches the tools the host positively knows are safe to repeat.
    ///
    /// List a tool here only when calling it twice is *definitionally*
    /// harmless — a pure read. Do **not** build this by inverting
    /// `Tool::external_effect()`: that signal is arg-less, and a tool whose
    /// effect depends on its arguments (`ShellTool`, raw tools)
    /// overrides `external_effect_with_args` while leaving the arg-less
    /// variant at the default `false`. Inverting it would silently admit
    /// exactly the tools that most need excluding.
    pub fn with_retry_safe_tools(mut self, tools: Arc<HashSet<String>>) -> Self {
        self.retry_safe_tools = Some(tools);
        self
    }

    /// Whether repeating `name` after a timeout is safe.
    ///
    /// Membership is the only way to be safe: an unlisted tool — unknown,
    /// arg-sensitive, or simply new — is treated as potentially effectful.
    fn timeout_is_retry_safe(&self, name: &str) -> bool {
        self.retry_safe_tools
            .as_deref()
            .is_some_and(|safe| safe.contains(name))
    }

    /// Projects one OpenHuman failure class onto the crate's coarse
    /// [`OutcomeClass`].
    ///
    /// Exhaustive on purpose — no `_` arm. A new [`ToolFailureClass`] must not
    /// silently inherit some neighbour's retry verdict; adding one should break
    /// this build and force the author to decide whether repeating the call is
    /// safe.
    ///
    /// * `ServiceUnavailable` and `ModelConnection` →
    ///   [`OutcomeClass::RetryableFailure`]. Both mean the request did not reach
    ///   a handler, so repeating it cannot duplicate an effect.
    /// * `Timeout` → retryable **only** when `retry_safe`. A timeout is the one
    ///   class where failure and "succeeded, but the reply was lost" look
    ///   identical: an email send, a payment, or a shell command may already
    ///   have committed. Repeating that duplicates a side effect the user never
    ///   asked for twice, so the verdict defers to the host's
    ///   `Tool::external_effect()` declaration and stays permanent whenever it
    ///   is unavailable.
    /// * Everything else → [`OutcomeClass::PermanentFailure`]. Missing
    ///   permissions, a missing app, and bad credentials need a human to act, so
    ///   an identical re-dispatch just burns an iteration; `BlockedByPolicy`,
    ///   `Denied`, and `ApprovalExpired` are refusals that auto-retrying would
    ///   actively subvert (#4459); and `Unknown` is the case where OpenHuman has
    ///   no basis to promise a repeat is safe.
    fn class_of(failure: ToolFailureClass, retry_safe: bool) -> OutcomeClass {
        match failure {
            ToolFailureClass::Timeout if retry_safe => OutcomeClass::RetryableFailure,
            ToolFailureClass::Timeout => OutcomeClass::PermanentFailure,

            ToolFailureClass::ServiceUnavailable | ToolFailureClass::ModelConnection => {
                OutcomeClass::RetryableFailure
            }

            ToolFailureClass::MissingPermission
            | ToolFailureClass::MissingApp
            | ToolFailureClass::BadCredentials
            | ToolFailureClass::BlockedByPolicy
            | ToolFailureClass::Denied
            | ToolFailureClass::ApprovalExpired
            | ToolFailureClass::Unknown => OutcomeClass::PermanentFailure,
        }
    }

    /// Joins `error` and `content` into the text the heuristic reads.
    ///
    /// Mirrors `TinyAgentsToolStatusMiddleware::after_tool` exactly (#4459): the
    /// classifier historically read `error` while the marker/timeout sniffs read
    /// `content`, and the two disagreeing is the bug that combination fixed.
    /// Borrows wherever possible so the common single-source case allocates
    /// nothing on the hot path.
    fn failure_text<'a>(result: &'a ToolResult) -> Cow<'a, str> {
        let error = result.error.as_deref().unwrap_or("");
        if error.is_empty() {
            Cow::Borrowed(result.content.as_str())
        } else if result.content.is_empty() || result.content == error {
            Cow::Borrowed(error)
        } else {
            Cow::Owned(format!("{error}\n{}", result.content))
        }
    }
}

impl ToolOutcomeClassifier for OpenHumanToolOutcomeClassifier {
    fn classify(&self, name: &str, result: &ToolResult) -> OutcomeClass {
        // `error.is_none()` is the sole success signal, matching the middleware.
        // A tool that writes "Error: …" into `content` while leaving `error`
        // unset has reported success; second-guessing that here would make the
        // two paths disagree about whether the call failed at all.
        if result.error.is_none() {
            return OutcomeClass::Success;
        }

        let text = Self::failure_text(result);
        // No executor deadline flag reaches the runtime, so derive it the way
        // the middleware does. `classify` short-circuits the policy markers
        // ahead of this flag, so a TTL-expired approval whose reason literally
        // contains "timed out" still classifies as `ApprovalExpired`, never a
        // retryable `Timeout` (#4459).
        let timed_out = text.contains("timed out");
        let failure = classify(&text, timed_out);
        let retry_safe = self.timeout_is_retry_safe(name);
        let outcome = Self::class_of(failure.class, retry_safe);

        tracing::debug!(
            target: "tinyagents",
            tool = %name,
            class = ?failure.class,
            retry_safe,
            ?outcome,
            "[tinyagents::host] classified tool outcome"
        );
        outcome
    }
}

// TODO(phase4): rate limits (`429`, "rate limit", "too many requests",
// "retry after") and DNS blips ("dns error", "failed to resolve") currently fall
// through `tool_status::classify` to `Unknown` and therefore land on
// `PermanentFailure` here, even though they are textbook retryable. The phrase
// list that does recognise them is `is_recoverable_tool_failure`, a private fn in
// `src/openhuman/tinyagents/middleware.rs`. Copying it into this adapter would
// fork the heuristic; the fix is to move those needles into
// `tool_status::ops::classify_class` (a `RateLimited` class, or extending
// `ServiceUnavailable`) and let both callers read one list. Not done here because
// Phase 4 must not edit existing files.

#[cfg(test)]
#[path = "tool_outcome_classifier_tests.rs"]
mod tests;
