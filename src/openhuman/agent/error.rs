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
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn display_formatting() {
        let err = AgentError::MaxIterationsExceeded { max: 10 };
        assert_eq!(
            err.to_string(),
            "Agent exceeded maximum tool iterations (10)"
        );

        let err = AgentError::CostBudgetExceeded {
            spent_microdollars: 5_500_000,
            budget_microdollars: 5_000_000,
        };
        assert!(err.to_string().contains("5.5000"));
    }

    #[test]
    fn context_limit_detection() {
        assert!(is_context_limit_error("prompt is too long for model"));
        assert!(is_context_limit_error("context_length_exceeded"));
        assert!(!is_context_limit_error("rate limit exceeded"));
    }

    #[test]
    fn max_iterations_detection_matches_display() {
        // The substring helper must match the variant's own Display output —
        // the channels dispatch / web_channel sites flatten the typed error
        // through a `String` boundary, so any drift between the constant
        // and `Display` silently re-enables Sentry emission for the cap
        // (OPENHUMAN-TAURI-99 / -98).
        let rendered = AgentError::MaxIterationsExceeded { max: 8 }.to_string();
        assert!(is_max_iterations_error(&rendered));
        assert!(is_max_iterations_error(
            "run_chat_task failed client_id=abc thread_id=t1 \
             error=Agent exceeded maximum tool iterations (10)"
        ));
        assert!(!is_max_iterations_error("provider returned 503"));
        assert!(!is_max_iterations_error(
            "Tool execution error [shell]: denied"
        ));
    }

    #[test]
    fn permission_denied_display() {
        let err = AgentError::PermissionDenied {
            tool_name: "shell".into(),
            required_level: "Execute".into(),
            channel_max_level: "ReadOnly".into(),
        };
        assert!(err.to_string().contains("shell"));
        assert!(err.to_string().contains("Execute"));
    }

    #[test]
    fn display_formats_other_variants() {
        assert!(AgentError::ProviderError {
            message: "boom".into(),
            retryable: true,
        }
        .to_string()
        .contains("retryable=true"));
        assert!(AgentError::ContextLimitExceeded {
            utilization_pct: 98
        }
        .to_string()
        .contains("98% utilized"));
        assert!(AgentError::ToolExecutionError {
            tool_name: "shell".into(),
            message: "denied".into(),
        }
        .to_string()
        .contains("Tool execution error [shell]"));
        assert!(AgentError::CompactionFailed {
            message: "summary failed".into(),
            consecutive_failures: 3,
        }
        .to_string()
        .contains("3 consecutive"));
    }

    #[test]
    fn from_anyhow_recovers_typed_agent_error_and_other_source() {
        let typed = anyhow::anyhow!(AgentError::MaxIterationsExceeded { max: 4 });
        match AgentError::from(typed) {
            AgentError::MaxIterationsExceeded { max } => assert_eq!(max, 4),
            other => panic!("unexpected variant: {other}"),
        }

        let other = AgentError::from(anyhow::anyhow!("plain failure"));
        assert!(matches!(other, AgentError::Other(_)));
        assert!(other.source().is_some());
    }

    // ── AgentError::EmptyProviderResponse (TAURI-RUST-4JX) ──────────────────
    //
    // `agent::harness::session::turn` returns this variant when the provider's
    // chat completion contains no text, no thinking, and no tool calls (a
    // degenerate/poisoned response — typically a flaky local model). The
    // variant was added so `run_single` can route it through `skips_sentry()`
    // and demote like `MaxIterationsExceeded`, keeping TAURI-RUST-4JX off
    // Sentry while preserving the user-visible error and the `Err` propagation
    // contract.

    #[test]
    fn empty_provider_response_display_matches_user_facing_string() {
        // The exact wire string is anchored: the UI surfaces it verbatim to
        // the user, and the emit-site comment at
        // `agent/harness/session/turn.rs:801` (the warn breadcrumb) explicitly
        // calls out the "surfacing as error instead of a silent blank reply"
        // contract. Any change to this byte string is a user-visible message
        // change and a Sentry-fingerprint change.
        let err = AgentError::EmptyProviderResponse { iteration: 1 };
        assert_eq!(
            err.to_string(),
            "The model returned an empty response. Please try again."
        );
    }

    #[test]
    fn skips_sentry_returns_true_for_known_user_state_variants() {
        // The two variants that represent user/provider state rather than a
        // code bug — `run_single` suppresses both from Sentry while still
        // returning `Err` so the user sees the failure.
        assert!(AgentError::MaxIterationsExceeded { max: 10 }.skips_sentry());
        assert!(AgentError::EmptyProviderResponse { iteration: 1 }.skips_sentry());
    }

    #[test]
    fn skips_sentry_returns_false_for_real_failures() {
        // Every other variant represents either an actionable bug, an
        // upstream provider/network failure that triage cares about, or a
        // CompactionFailed that already has its own follow-up logic — none
        // of them should silently disappear from Sentry.
        let real_failures = [
            AgentError::ProviderError {
                message: "boom".into(),
                retryable: true,
            },
            AgentError::ContextLimitExceeded {
                utilization_pct: 98,
            },
            AgentError::ToolExecutionError {
                tool_name: "shell".into(),
                message: "denied".into(),
            },
            AgentError::CostBudgetExceeded {
                spent_microdollars: 1_000,
                budget_microdollars: 500,
            },
            AgentError::CompactionFailed {
                message: "summary failed".into(),
                consecutive_failures: 2,
            },
            AgentError::PermissionDenied {
                tool_name: "shell".into(),
                required_level: "Execute".into(),
                channel_max_level: "ReadOnly".into(),
            },
            AgentError::Other(anyhow::anyhow!("plain failure")),
        ];
        for err in real_failures {
            assert!(!err.skips_sentry(), "must NOT skip Sentry for: {err}");
        }
    }
}
