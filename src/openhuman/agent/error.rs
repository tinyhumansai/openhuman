//! Structured error types for the agent loop.
//!
//! Replaces generic `anyhow::bail!` with typed variants so callers can
//! distinguish retryable errors from permanent failures and take appropriate
//! recovery actions (e.g. triggering compaction on context-limit errors).

use std::fmt;

/// Structured error type for agent loop operations.
#[derive(Debug)]
pub enum AgentError {
    /// The LLM provider returned an error (e.g., API key invalid, network failure).
    /// `retryable` indicates if the operation should be attempted again.
    ProviderError { message: String, retryable: bool },

    /// Context window is exhausted and compaction/summarization cannot help.
    /// The agent cannot proceed without dropping significant history.
    ContextLimitExceeded { utilization_pct: u8 },

    /// A tool execution failed during its `execute()` method.
    ToolExecutionError { tool_name: String, message: String },

    /// The daily cost budget for this user/agent has been exceeded.
    /// Prevents unexpected runaway costs.
    CostBudgetExceeded {
        spent_microdollars: u64,
        budget_microdollars: u64,
    },

    /// The agent exceeded its maximum allowed tool iterations for a single turn.
    /// Typically indicates an infinite loop in the model's reasoning.
    MaxIterationsExceeded { max: usize },

    /// The provider's chat completion contained no text, no thinking, and
    /// no tool calls — a degenerate / poisoned response. Typically observed
    /// with flaky local model fine-tunes (e.g. community quantizations of
    /// Qwen/Llama via LM Studio or Ollama). Surfaced as a user-facing
    /// error instead of a silent blank reply (defense-in-depth from
    /// `agent/harness/session/turn.rs`) but suppressed from Sentry — it's
    /// a provider/user-state outcome, not an OpenHuman bug, and a deeper
    /// fix lives in the model / provider config the user chose. Targets
    /// Sentry TAURI-RUST-4JX (~33 events, escalating on 0.56.0).
    EmptyProviderResponse { iteration: usize },

    /// Automated history compaction (summarization) failed.
    CompactionFailed {
        message: String,
        consecutive_failures: u8,
    },

    /// The current channel (e.g., Telegram) does not have permission to execute
    /// the requested tool (e.g., shell access).
    PermissionDenied {
        tool_name: String,
        required_level: String,
        channel_max_level: String,
    },

    /// The tinyagents `CapabilityRegistry` produced one or more error-severity
    /// diagnostics while projecting the turn's tool/model/graph surface (e.g. a
    /// duplicate tool name across native/MCP/Composio/generated tools or a
    /// dangling alias). The turn is aborted fail-closed *before* the first model
    /// dispatch so no provider call runs against an ambiguous registry.
    RegistryValidationFailed { diagnostics: Vec<String> },

    /// Generic/untyped error (escape hatch for migration or external dependencies).
    Other(anyhow::Error),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderError { message, retryable } => {
                write!(f, "Provider error (retryable={retryable}): {message}")
            }
            Self::ContextLimitExceeded { utilization_pct } => {
                write!(
                    f,
                    "Context window exhausted ({utilization_pct}% utilized, compaction disabled)"
                )
            }
            Self::ToolExecutionError { tool_name, message } => {
                write!(f, "Tool execution error [{tool_name}]: {message}")
            }
            Self::CostBudgetExceeded {
                spent_microdollars,
                budget_microdollars,
            } => {
                let spent = *spent_microdollars as f64 / 1_000_000.0;
                let budget = *budget_microdollars as f64 / 1_000_000.0;
                write!(
                    f,
                    "Daily cost budget exceeded: spent ${spent:.4}, budget ${budget:.4}"
                )
            }
            Self::MaxIterationsExceeded { max } => {
                write!(f, "{MAX_ITERATIONS_ERROR_PREFIX} ({max})")
            }
            Self::EmptyProviderResponse { .. } => {
                // Verbatim user-facing string from the old
                // `agent/harness/session/turn.rs` emit site — UI / tests
                // grep for this exact byte sequence.
                write!(f, "The model returned an empty response. Please try again.")
            }
            Self::CompactionFailed {
                message,
                consecutive_failures,
            } => {
                write!(
                    f,
                    "Compaction failed ({consecutive_failures} consecutive): {message}"
                )
            }
            Self::PermissionDenied {
                tool_name,
                required_level,
                channel_max_level,
            } => {
                write!(
                    f,
                    "Permission denied for tool '{tool_name}': requires {required_level}, channel allows {channel_max_level}"
                )
            }
            Self::RegistryValidationFailed { diagnostics } => {
                write!(
                    f,
                    "Capability registry validation failed ({} error diagnostic(s)): {}",
                    diagnostics.len(),
                    diagnostics.join("; ")
                )
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl AgentError {
    /// User/provider-state outcomes that the UI already surfaces to the
    /// user and that no developer can act on from Sentry — `run_single`
    /// suppresses their Sentry emission (`log::info!` only) while still
    /// returning the `Err` so the existing `AgentError` + `recoverable`
    /// semantics are preserved.
    ///
    /// - `MaxIterationsExceeded`: deterministic tool-loop cap, drives
    ///   OPENHUMAN-TAURI-99 / -98 suppression.
    /// - `EmptyProviderResponse`: degenerate/poisoned chat completion,
    ///   drives TAURI-RUST-4JX suppression.
    ///
    /// Other variants are real failures (`ProviderError` upstream HTTP /
    /// network, `ToolExecutionError` callable bug, `ContextLimitExceeded`
    /// compaction gap, `CostBudgetExceeded`, `CompactionFailed`,
    /// `PermissionDenied` config bug, `Other` escape hatch) and must
    /// continue to escalate.
    pub fn skips_sentry(&self) -> bool {
        matches!(
            self,
            Self::MaxIterationsExceeded { .. } | Self::EmptyProviderResponse { .. }
        )
    }
}

impl From<anyhow::Error> for AgentError {
    fn from(e: anyhow::Error) -> Self {
        // Attempt to recover a typed AgentError that was wrapped in anyhow.
        match e.downcast::<AgentError>() {
            Ok(agent_err) => agent_err,
            Err(other) => Self::Other(other),
        }
    }
}

/// Canonical user-facing prefix for the max-tool-iterations cap.
///
/// Single source of truth for:
/// - `AgentError::MaxIterationsExceeded` `Display` (in this file)
/// - Substring detection at Sentry-emit funnels where the typed variant has
///   already been marshalled through `String` (channels dispatch path,
///   web-channel run_chat_task, optional `before_send` defense)
///
/// Keep the literal **exactly** in sync with the `Display` impl above — UI
/// surfaces and tests grep for this prefix.
pub const MAX_ITERATIONS_ERROR_PREFIX: &str = "Agent exceeded maximum tool iterations";

/// Returns true when an error rendering contains the canonical
/// max-tool-iterations cap message.
///
/// Use this at Sentry-emit sites (`channels::dispatch`, `web_channel::
/// run_chat_task`, and Sentry `before_send` filters) where the typed
/// [`AgentError::MaxIterationsExceeded`] variant has already been flattened
/// to a `String` by the native bus / handler boundary and cannot be
/// downcast directly. Sites that still hold an `anyhow::Error` should
/// prefer `err.downcast_ref::<AgentError>()` for precision.
pub fn is_max_iterations_error(error_msg: &str) -> bool {
    error_msg.contains(MAX_ITERATIONS_ERROR_PREFIX)
}

/// Check if an error message indicates a context/prompt-too-long failure.
pub fn is_context_limit_error(error_msg: &str) -> bool {
    let lower = error_msg.to_lowercase();
    lower.contains("prompt is too long")
        || lower.contains("context_length_exceeded")
        || lower.contains("maximum context length")
        || lower.contains("prompt too long")
        || lower.contains("token limit")
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
