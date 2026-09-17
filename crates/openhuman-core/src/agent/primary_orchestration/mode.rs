use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::OrchestrationEngine;

pub const LOCAL_QWEN_PROVIDER_BINDING: &str = "lmstudio:qwen38-openhuman";

/// User-visible execution shape selected before any provider call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PrimaryTurnMode {
    Chat,
    Assist,
    Agent,
}

impl PrimaryTurnMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Assist => "assist",
            Self::Agent => "agent",
        }
    }

    pub fn max_primary_calls(self) -> u32 {
        match self {
            Self::Chat => 1,
            Self::Assist => 3,
            Self::Agent => 12,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModeResolutionInput<'a> {
    pub user_message: &'a str,
    pub explicit_override: Option<PrimaryTurnMode>,
    pub has_live_agent_checkpoint: bool,
}

/// Resolve the turn mode locally and deterministically.
///
/// The ordered branches are the executable form of the Phase 7 precedence.
/// Phrase matching is deliberately confined to mode selection.  It does not
/// choose or authorize tools, and it cannot promote a turn based on whatever
/// tools happen to be registered.
pub fn resolve_primary_turn_mode(input: ModeResolutionInput<'_>) -> PrimaryTurnMode {
    if let Some(explicit) = input.explicit_override {
        return explicit;
    }

    let normalized = normalize(input.user_message);
    if input.has_live_agent_checkpoint && !explicitly_cancels(&normalized) {
        return PrimaryTurnMode::Agent;
    }

    if requests_durable_or_multistep_work(&normalized) {
        return PrimaryTurnMode::Agent;
    }
    if requests_bounded_action_or_current_data(&normalized) {
        return PrimaryTurnMode::Assist;
    }
    PrimaryTurnMode::Chat
}

/// Apply the Phase 7 rollout boundary.  Goose is enabled only for the exact
/// local-Qwen provider binding and only at the primary interactive entrypoint.
/// Setting the persisted engine to `tinyagents` is the one-release rollback.
pub fn resolve_orchestration_engine(
    configured: OrchestrationEngine,
    provider_binding: &str,
    primary_interactive: bool,
) -> OrchestrationEngine {
    if primary_interactive
        && provider_binding
            .trim()
            .eq_ignore_ascii_case(LOCAL_QWEN_PROVIDER_BINDING)
    {
        configured
    } else {
        OrchestrationEngine::Tinyagents
    }
}

fn normalize(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn explicitly_cancels(message: &str) -> bool {
    ordered_phrase_match(
        message,
        &[
            "cancel the task",
            "cancel that task",
            "cancel the run",
            "cancel that",
            "stop working",
            "stop the task",
            "abort the task",
            "never mind",
            "nevermind",
        ],
    )
}

fn requests_durable_or_multistep_work(message: &str) -> bool {
    ordered_phrase_match(
        message,
        &[
            "autonomously",
            "do not stop",
            "don't stop",
            "keep working until",
            "work until",
            "end to end",
            "end-to-end",
            "multi-step",
            "multiple steps",
            "create a goal",
            "continue the goal",
            "delegate this",
            "spawn an agent",
            "schedule ",
            "set a reminder",
            "monitor ",
            "keep an eye on",
            "watch for ",
            "edit the repository",
            "change the repository",
            "modify the repository",
            "update the repository",
            "fix the code",
            "implement ",
            "refactor ",
            "run the tests",
            "run tests",
            "build the project",
            "change the readme",
            "edit the readme",
        ],
    )
}

fn requests_bounded_action_or_current_data(message: &str) -> bool {
    ordered_phrase_match(
        message,
        &[
            "browse ",
            "search the web",
            "search online",
            "look up ",
            "find online",
            "current news",
            "latest news",
            "today's news",
            "todays news",
            "headlines",
            "fetch ",
            "open http://",
            "open https://",
            "http://",
            "https://",
            "from the internet",
            "generate an image",
            "generate a portrait",
            "create an image",
            "draw an image",
            "edit this image",
            "remember that",
            "remember this",
            "recall ",
            "what do you remember",
            "send ",
            "download ",
            "execute ",
            "run this command",
        ],
    )
}

fn ordered_phrase_match(message: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| message.contains(phrase))
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod mode_tests;
