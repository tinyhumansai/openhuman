//! `run_single` / `run_interactive`: the single-shot and CLI entry points
//! that wrap [`OpenHumanSessionHost::turn`] with prompt-injection enforcement, telemetry
//! events, and error sanitisation.

use super::super::types::OpenHumanSessionHost;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use anyhow::Result;
use tinyagents_harness::agent_error::AgentError;

impl OpenHumanSessionHost {
    // ─────────────────────────────────────────────────────────────────
    // Run helpers — single-shot and interactive loops
    // ─────────────────────────────────────────────────────────────────

    /// Runs a single turn with the given message and returns the response.
    ///
    /// This is the primary high-level method for programmatic interaction with the agent.
    /// It wraps the core `turn` logic with telemetry events (`AgentTurnStarted`,
    /// `AgentTurnCompleted`) and error sanitization.
    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        self.run_single_with_origin(message, None).await
    }

    /// Runs a single turn with authority supplied explicitly by its entry point.
    pub async fn run_single_with_origin(
        &mut self,
        message: &str,
        origin: Option<crate::agent::turn_origin::AgentTurnOrigin>,
    ) -> Result<String> {
        self.run_single_with_approval(message, origin, Default::default())
            .await
    }

    /// [`Self::run_single_with_origin`] for a turn that also owns an approval
    /// context (chat surface, copilot flag, flow run). The context is handed to
    /// this turn alone.
    pub async fn run_single_with_approval(
        &mut self,
        message: &str,
        origin: Option<crate::agent::turn_origin::AgentTurnOrigin>,
        approval: crate::security::approval::ApprovalTurnContext,
    ) -> Result<String> {
        let origin = origin.or_else(crate::core::runtime::CoreContext::current_turn_origin);
        let guard = enforce_prompt_input(
            message,
            PromptEnforcementContext {
                source: "agent.runtime.run_single",
                request_id: None,
                user_id: Some(self.event_channel()),
                session_id: Some(self.event_session_id()),
            },
        );
        // Module scan: context only (can add a refusal, never lift one).
        let module_blocked = !self.untrusted_input
            && matches!(guard.action, PromptEnforcementAction::Allow)
            && crate::security::prompt_injection::module_scan(message)
                .await
                .is_some_and(|scan| scan.blocks());
        // A host-only session reading untrusted data has opted out; see
        // `set_untrusted_input`, which refuses any other session.
        if module_blocked
            || (!self.untrusted_input && !matches!(guard.action, PromptEnforcementAction::Allow))
        {
            let user_message = match guard.action {
                PromptEnforcementAction::Allow if module_blocked => {
                    "Prompt blocked by security policy."
                }
                PromptEnforcementAction::Allow => "Message accepted.",
                PromptEnforcementAction::Blocked => "Prompt blocked by security policy.",
                PromptEnforcementAction::ReviewBlocked => {
                    "Prompt flagged for security review and was not processed."
                }
            };
            let action_tag = match guard.action {
                PromptEnforcementAction::Allow if module_blocked => "module_blocked",
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "blocked",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            };
            crate::core::observability::report_error(
                user_message,
                "agent",
                "prompt_injection_blocked",
                &[
                    ("session_id", self.event_session_id()),
                    ("channel", self.event_channel()),
                    ("action", action_tag),
                ],
            );
            BUS.publish(DomainEvent::AgentError {
                session_id: self.event_session_id().to_string(),
                message: user_message.to_string(),
                recoverable: true,
            });
            return Err(anyhow::anyhow!(user_message));
        }

        let history_snapshot = self.history();
        // Busy state for background-result delivery is marked here, in the
        // turn's own (profile) scope; the bus subscriber runs off-task.
        // A guard, so a turn dropped mid-flight does not leave it busy.
        let busy =
            crate::agent::orchestration::busy_guard::TurnBusy::start(self.event_session_id());
        BUS.publish(DomainEvent::AgentTurnStarted {
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        });

        match self
            .turn_with_approval(message, origin.as_ref(), approval)
            .await
        {
            Ok(response) => {
                let history = self.history();
                let new_entries = Self::new_entries_for_turn(&history_snapshot, &history);
                BUS.publish(DomainEvent::AgentTurnCompleted {
                    session_id: self.event_session_id().to_string(),
                    text_chars: response.chars().count(),
                    iterations: Self::count_iterations(new_entries),
                });
                drop(busy);
                Ok(response)
            }
            Err(err) => {
                let sanitized_message = Self::sanitize_event_error_message(&err);
                // Some typed `AgentError` variants represent agent / user /
                // provider state that the UI already surfaces — the
                // max-tool-iterations cap (OPENHUMAN-TAURI-99 / -98,
                // chat-rendered "Error: OpenHumanSessionHost exceeded maximum tool
                // iterations") and the empty-provider-response degeneracy
                // (TAURI-RUST-4JX, "The model returned an empty response.
                // Please try again."). Skip the Sentry funnel for both
                // and emit a structured `log::info!` instead. The
                // suppressed set is owned by `AgentError::is_expected_outcome()`
                // so the policy stays in one place.
                //
                // Other agent errors go through `report_error_or_expected`
                // so OPENHUMAN-TAURI-5Z and the budget-noise cluster —
                // upstream transient HTTP and backend budget-exhausted 400s
                // that bubble up under `domain=agent` and escape the
                // `domain=llm_provider` filter — get demoted to a
                // warn/info-level breadcrumb without losing genuine bugs.
                // `Err` propagation, the `AgentError` domain event, and
                // downstream `recoverable=false` semantics are preserved.
                let skips_sentry = err
                    .downcast_ref::<AgentError>()
                    .is_some_and(AgentError::is_expected_outcome);
                if skips_sentry {
                    log::info!(
                        target: "agent",
                        "[agent.run_single] suppressed Sentry emission for user-state agent error \
                         session_id={} channel={} error_kind={} message={}",
                        self.event_session_id(),
                        self.event_channel(),
                        sanitized_message.as_str(),
                        err
                    );
                } else {
                    crate::core::observability::report_error_or_expected(
                        &err,
                        "agent",
                        "run_single",
                        &[
                            ("session_id", self.event_session_id()),
                            ("channel", self.event_channel()),
                            ("error_kind", sanitized_message.as_str()),
                        ],
                    );
                }
                BUS.publish(DomainEvent::AgentError {
                    session_id: self.event_session_id().to_string(),
                    message: sanitized_message,
                    recoverable: false,
                });
                drop(busy);
                Err(err)
            }
        }
    }
}
