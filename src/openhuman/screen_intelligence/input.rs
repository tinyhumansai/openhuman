//! Input actions and autocomplete helpers for the screen intelligence session.

use super::helpers::{generate_suggestions, truncate_tail, validate_input_action};
use super::limits::{MAX_CONTEXT_CHARS, MAX_SUGGESTION_CHARS};
use super::state::AccessibilityEngine;
use super::types::{
    AutocompleteCommitParams, AutocompleteCommitResult, AutocompleteSuggestParams,
    AutocompleteSuggestResult, InputActionParams, InputActionResult,
};
use crate::openhuman::accessibility::foreground_context;

impl AccessibilityEngine {
    pub async fn input_action(
        &self,
        action: InputActionParams,
    ) -> Result<InputActionResult, String> {
        let mut state = self.inner.lock().await;

        if action.action == "panic_stop" {
            drop(state);
            self.stop_session_internal("panic_stop".to_string()).await;
            return Ok(InputActionResult {
                accepted: true,
                blocked: false,
                reason: Some("panic stop executed".to_string()),
            });
        }

        if state.session.is_none() {
            return Ok(InputActionResult {
                accepted: false,
                blocked: true,
                reason: Some("session is not active".to_string()),
            });
        }

        let context = foreground_context();
        if let Some(ctx) = &context {
            if !self.should_capture_context(ctx, &state.config) {
                return Ok(InputActionResult {
                    accepted: false,
                    blocked: true,
                    reason: Some("action blocked by denylisted context".to_string()),
                });
            }
        }

        validate_input_action(&action)?;

        if let Some(text) = action.text.as_ref() {
            if !text.is_empty() {
                if !state.autocomplete_context.is_empty() {
                    state.autocomplete_context.push(' ');
                }
                state.autocomplete_context.push_str(text);
                state.autocomplete_context =
                    truncate_tail(&state.autocomplete_context, MAX_CONTEXT_CHARS);
            }
        }

        let action_name = action.action.clone();
        state.last_event = Some(format!("input_action:{action_name}"));

        Ok(InputActionResult {
            accepted: true,
            blocked: false,
            reason: None,
        })
    }

    pub async fn autocomplete_suggest(
        &self,
        params: AutocompleteSuggestParams,
    ) -> Result<AutocompleteSuggestResult, String> {
        let state = self.inner.lock().await;

        if !state.config.autocomplete_enabled {
            return Ok(AutocompleteSuggestResult {
                suggestions: Vec::new(),
            });
        }

        let mut context = params.context.unwrap_or_default();
        if context.trim().is_empty() {
            context = state.autocomplete_context.clone();
        }
        drop(state);

        let max_results = params.max_results.unwrap_or(3).clamp(1, 8);
        let suggestions = generate_suggestions(&context, max_results);

        Ok(AutocompleteSuggestResult { suggestions })
    }

    pub async fn autocomplete_commit(
        &self,
        params: AutocompleteCommitParams,
    ) -> Result<AutocompleteCommitResult, String> {
        let cleaned = params.suggestion.trim();
        if cleaned.is_empty() {
            return Err("suggestion cannot be empty".to_string());
        }
        if cleaned.len() > MAX_SUGGESTION_CHARS {
            return Err("suggestion exceeds maximum length".to_string());
        }

        let mut state = self.inner.lock().await;
        if !state.config.autocomplete_enabled {
            return Ok(AutocompleteCommitResult { committed: false });
        }
        if !state.autocomplete_context.is_empty() {
            state.autocomplete_context.push(' ');
        }
        state.autocomplete_context.push_str(cleaned);
        state.autocomplete_context = truncate_tail(&state.autocomplete_context, MAX_CONTEXT_CHARS);
        state.last_event = Some("autocomplete_commit".to_string());

        Ok(AutocompleteCommitResult { committed: true })
    }
}
