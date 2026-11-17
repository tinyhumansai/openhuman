//! OpenHuman's timeout-safety policy over TinyAgents' shared status classifier.
//!
//! TinyAgents owns error extraction, classification, policy-marker precedence,
//! and the mapping to [`OutcomeClass`]. OpenHuman supplies only the current
//! host's allowlist of tool names that are safe to repeat after a timeout.

use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_harness::host::{OutcomeClass, ToolOutcomeClassifier, ToolStatusClassifier};
use tinytools::ToolResult;

/// OpenHuman's [`ToolOutcomeClassifier`], backed by
/// [`tinytools::status::classify`].
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
    /// Only consulted for timeouts, where repeating may duplicate an effect.
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
}

impl ToolOutcomeClassifier for OpenHumanToolOutcomeClassifier {
    fn classify(&self, name: &str, result: &ToolResult) -> OutcomeClass {
        let retry_safe_tools = self.retry_safe_tools.clone();
        ToolStatusClassifier
            .with_retry_safe_timeouts(move |tool| {
                retry_safe_tools
                    .as_deref()
                    .is_some_and(|safe| safe.contains(tool))
            })
            .classify(name, result)
    }
}

#[cfg(test)]
#[path = "tool_outcome_classifier_tests.rs"]
mod tests;
