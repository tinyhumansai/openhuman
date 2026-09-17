//! `run_single` / `run_interactive`: the single-shot and CLI entry points
//! that wrap [`Agent::turn`] with prompt-injection enforcement, telemetry
//! events, and error sanitisation.

use super::super::types::Agent;
use crate::agent::error::AgentError;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use anyhow::Result;

impl Agent {
    pub(super) fn begin_guarded_run(
        &self,
        message: &str,
    ) -> Result<Vec<crate::agent::messages::ConversationMessage>> {
        // ─────────────────────────────────────────────────────────────────
        let guard = enforce_prompt_input(
            message,
            PromptEnforcementContext {
                source: "agent.runtime.run_single",
                request_id: None,
                user_id: Some(self.event_channel()),
                session_id: Some(self.event_session_id()),
            },
        );
        if !matches!(guard.action, PromptEnforcementAction::Allow) {
            let user_message = match guard.action {
                PromptEnforcementAction::Allow => "Message accepted.",
                PromptEnforcementAction::Blocked => "Prompt blocked by security policy.",
                PromptEnforcementAction::ReviewBlocked => {
                    "Prompt flagged for security review and was not processed."
                }
            };
            let action_tag = match guard.action {
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

        let history_snapshot = self.history.clone();
        BUS.publish(DomainEvent::AgentTurnStarted {
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        });
        Ok(history_snapshot)
    }

    pub(super) fn finish_guarded_run(
        &self,
        history_snapshot: &[crate::agent::messages::ConversationMessage],
        result: Result<String>,
    ) -> Result<String> {
        match result {
            Ok(response) => {
                let new_entries = Self::new_entries_for_turn(history_snapshot, &self.history);
                BUS.publish(DomainEvent::AgentTurnCompleted {
                    session_id: self.event_session_id().to_string(),
                    text_chars: response.chars().count(),
                    iterations: Self::count_iterations(new_entries),
                });
                Ok(response)
            }
            Err(err) => {
                let sanitized_message = Self::sanitize_event_error_message(&err);
                // Some typed `AgentError` variants represent agent / user /
                // provider state that the UI already surfaces — the
                // max-tool-iterations cap (OPENHUMAN-TAURI-99 / -98,
                // chat-rendered "Error: Agent exceeded maximum tool
                // iterations") and the empty-provider-response degeneracy
                // (TAURI-RUST-4JX, "The model returned an empty response.
                // Please try again."). Skip the Sentry funnel for both
                // and emit a structured `log::info!` instead. The
                // suppressed set is owned by `AgentError::skips_sentry()`
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
                    .is_some_and(AgentError::skips_sentry);
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
                Err(err)
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Run helpers — single-shot and interactive loops
    // ─────────────────────────────────────────────────────────────────

    /// Runs a single turn with the given message and returns the response.
    ///
    /// This is the primary high-level method for programmatic interaction with the agent.
    /// It wraps the core `turn` logic with telemetry events (`AgentTurnStarted`,
    /// `AgentTurnCompleted`) and error sanitization.
    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        let history_snapshot = self.begin_guarded_run(message)?;
        let result = self.turn(message).await;
        self.finish_guarded_run(&history_snapshot, result)
    }

    /// Runs an interactive CLI loop, reading from standard input and printing to standard output.
    ///
    /// This method starts a persistent session where the user can chat with the agent
    /// directly from the console. It handles input until a termination command
    /// (e.g., `/quit`) is received.
    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            match self.run_single(&msg.content).await {
                Ok(response) => println!("\n{response}\n"),
                Err(e) => {
                    // `run_single` already publishes `AgentError` and
                    // sanitises the payload; surface a concise line here
                    // for the CLI user and continue the loop.
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            }
        }

        listen_handle.abort();
        Ok(())
    }
}
