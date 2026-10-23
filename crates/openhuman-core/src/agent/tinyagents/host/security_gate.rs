//! Host [`SecurityGate`] for a hosted root — what is left of it once tool
//! authorization moved onto the one shared gate (issue #7328, phase 6).
//!
//! The tinyagents runtime holds *no* authority: it asks, and obeys. A tool call
//! is authorized by the security middleware every agent path installs first
//! (`middleware::security_gate`): the TinySecurity module answers for the call's
//! tier, origin, permission level, hourly budget and shell class, the channel
//! tool policy feeds its "ask a human first" tools into the same gate, and the
//! one approval resolver parks whatever needs a human. A hosted root therefore
//! has no second decision to make and no second approval to raise.
//!
//! This adapter keeps only the two answers the middleware cannot give:
//!
//! - **A nested call is refused.** A call made by a running tool, not the model,
//!   can never be parked for a human, and the middleware stack is the model's
//!   path. Nested calls are off (`RunLimits::max_nested_depth = 0`) and stay
//!   refused here, failing closed, until the host decides how to authorize them.
//! - **Untrusted text is screened.** [`SecurityGate::screen_input`] asks the
//!   security module's prompt screen (through the scanner port) about text before
//!   it enters the model context. A screen that cannot answer in time holds the
//!   text for review, so the hot path blocks rather than waves content through.
//!
//! No `Redacted` outcome is ever produced. OpenHuman can *detect* PII
//! ([`tinymemory_api::safety::scan`]) and has a module-backed free-text redactor
//! (`security::redact::text`), but mapping "PII found, injection clean" to
//! `Redacted` is a behaviour change owned by the phase 4 approval move. Screening
//! therefore only ever passes or blocks. See the `TODO(phase4)` in
//! [`OpenHumanSecurityGate::screen_input`].

use async_trait::async_trait;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::host::security_gate::{
    ContentOrigin, GateDecision, ScreenOutcome, SecurityGate, ToolCallRequest,
};

use crate::agent::tinyagents::policy_denial::PolicyDenial;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

/// OpenHuman's [`SecurityGate`].
pub struct OpenHumanSecurityGate {
    /// `false` for a host-only turn reading untrusted data
    /// ([`Self::with_input_screening`]): its input is data, not instructions.
    screens_input: bool,
}

impl Default for OpenHumanSecurityGate {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenHumanSecurityGate {
    /// Builds a gate that screens every origin's input.
    pub fn new() -> Self {
        Self {
            screens_input: true,
        }
    }

    /// Whether this gate screens the turn's input. A host-only turn reading
    /// untrusted data (`OpenHumanRunContext::untrusted_input_turn`) passes
    /// `false`: its input is data, not instructions.
    pub fn with_input_screening(mut self, screens_input: bool) -> Self {
        self.screens_input = screens_input;
        self
    }
}

#[async_trait]
impl SecurityGate for OpenHumanSecurityGate {
    /// Refuses a nested call; every other call is authorized by the security
    /// middleware on the model's tool path, so there is nothing more to decide
    /// here.
    async fn authorize_tool(&self, call: &ToolCallRequest) -> TaResult<GateDecision> {
        if call.is_nested() {
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                "[tinyagents::host::security] nested tool call refused (fail closed)"
            );
            return Ok(GateDecision::deny(
                PolicyDenial::PolicyDenied {
                    tool: &call.tool_name,
                    policy: "nested tool calls",
                    reason: "Tools may not call other tools in this session.",
                }
                .render(),
            ));
        }
        Ok(GateDecision::Allow)
    }

    /// Screens `text` with the security module's prompt scan.
    ///
    /// Every origin is screened, `User` included — the detector's primary
    /// production caller is the user-prompt path, and provenance is not trust.
    /// Both non-allow verdicts map to [`ScreenOutcome::Block`]: OpenHuman's
    /// `Review` verdict is already spelled `ReviewBlocked` on the enforcement
    /// side, so admitting it here would be this adapter inventing a permission
    /// the host does not grant.
    ///
    /// The reason string carries the verdict and the rule codes only — never
    /// the screened text, which is the thing suspected of being hostile.
    ///
    /// TODO(phase4): no [`ScreenOutcome::Redacted`] is ever returned. The
    /// free-text redactor now exists (`security::redact::text`, answered by the
    /// TinySecurity module); mapping "PII found, injection clean" to `Redacted`
    /// is a behaviour change that belongs with the phase 4 approval move.
    async fn screen_input(&self, text: &str, origin: ContentOrigin) -> TaResult<ScreenOutcome> {
        if !self.screens_input {
            return Ok(ScreenOutcome::Pass);
        }
        let source = match origin {
            ContentOrigin::User => "agent.user",
            ContentOrigin::Tool => "agent.tool_output",
            ContentOrigin::Web => "agent.web",
            ContentOrigin::Channel => "agent.channel",
            ContentOrigin::Agent => "agent.subagent",
            ContentOrigin::Stored => "agent.stored",
        };
        let decision = enforce_prompt_input(
            text,
            PromptEnforcementContext {
                source,
                request_id: None,
                user_id: None,
                session_id: None,
            },
        )
        .await;

        match decision.action {
            PromptEnforcementAction::Allow => Ok(ScreenOutcome::Pass),
            PromptEnforcementAction::Blocked | PromptEnforcementAction::ReviewBlocked => {
                let codes: Vec<&str> = decision.reasons.iter().map(|r| r.code.as_str()).collect();
                tracing::warn!(
                    target: "tinyagents",
                    %source,
                    score = decision.score,
                    codes = %codes.join(","),
                    chars = decision.prompt_chars,
                    "[tinyagents::host::security] blocked untrusted input"
                );
                Ok(ScreenOutcome::block(format!(
                    "Content from '{source}' was rejected by the prompt-injection screen \
                     ({}). It has not been added to the conversation.",
                    if codes.is_empty() {
                        "no rule detail".to_string()
                    } else {
                        codes.join(", ")
                    }
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "security_gate_tests.rs"]
mod tests;
