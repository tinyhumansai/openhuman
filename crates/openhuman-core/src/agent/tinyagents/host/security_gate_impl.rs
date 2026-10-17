use async_trait::async_trait;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::host::security_gate::{
    ContentOrigin, GateDecision, ScreenOutcome, SecurityGate, ToolCallRequest,
};

use super::OpenHumanSecurityGate;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

#[async_trait]
impl SecurityGate for OpenHumanSecurityGate {
    /// Answers in five stages, each of which can only narrow the answer:
    ///
    /// 1. the channel tool policy (`agent_tool_policy`),
    /// 2. tool resolution — an unregistered name is denied,
    /// 3. `shell` command classification, which supersedes stage 4 for that one
    ///    tool because it is strictly finer-grained,
    /// 4. the autonomy tier for acting tools (`can_act`) plus the non-mutating
    ///    rate-limit read, and
    /// 5. the approval park for anything the tier prompts on.
    ///
    /// **A human approval never short-circuits the remaining stages.** Approving
    /// at stage 1 settles the *channel* restriction and nothing else; the call
    /// still has to survive tool resolution, the shell classifier, and the
    /// autonomy tier. Returning straight after the park would have let a
    /// channel that merely asks for confirmation authorize a `shell` command the
    /// tier blocks, or an acting tool under `readonly` — turning the channel's
    /// second-strictest setting into an override of the user's own autonomy
    /// choice. A refusal at any stage is still terminal, and `channel_approved`
    /// is carried forward so a later prompting stage does not ask twice.
    async fn authorize_tool(&self, call: &ToolCallRequest) -> TaResult<GateDecision> {
        self.authorize_tool_with_facts(call, None).await
    }

    async fn authorize_tool_with_context(
        &self,
        call: &ToolCallRequest,
        context: Option<&serde_json::Value>,
    ) -> TaResult<GateDecision> {
        self.authorize_tool_with_facts(call, context).await
    }

    /// Runs OpenHuman's prompt-injection detector over `text`.
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
    /// TODO(phase4): no [`ScreenOutcome::Redacted`] is ever returned. Producing
    /// one needs a free-text redactor; `tinymemory_api::safety::scan` detects spans but
    /// returns no rewritten string, and `approval::redact::scrub_paths` is
    /// private and shaped for JSON argument maps. Either lift a public
    /// `redact_text(&str) -> String` out of `approval::redact` or add one to
    /// `tinymemory_api::safety`, then map "PII found, injection clean" to `Redacted`.
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
        );

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
