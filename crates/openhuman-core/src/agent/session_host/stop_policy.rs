//! The reply a user reads when the failure breaker stopped a turn and no
//! model-written close could be used.
//!
//! It used to open with "my tool calls were not making progress" whatever the
//! cause, quote the breaker's stop note (written for the model, not the user)
//! and dump every tool result verbatim; one such dump carried a user's profile
//! email and IP address. This one gives a single plain-language reason derived
//! from the stop note's failure class, collapses identical results into one
//! line with a count, and shows at most each tool's name and one short,
//! scrubbed error line. No tool output is reproduced.
//!
//! Localization: assistant text the core writes has no locale path yet (the
//! chat surface localizes only `chat_error` events, through
//! `inference::failure_copy`'s `copy_key`). The copy is therefore English, but
//! every reason is a [`StopReasonKind`] with a stable `turn_stop.*` key, so a
//! locale table can replace [`StopReasonKind::copy`] without touching the
//! classification.

use tinyagents_harness::close::CloseToolResult as CheckpointToolResult;
use tinyagents_harness::turn_stop::{CopyRequest, TurnStop, TurnStopCopyKey};

/// Longest error line shown for one tool, in characters.
const ERROR_LINE_CHARS: usize = 120;

/// Maps an unclassified breaker stop to the product's stable stop category.
fn stop_fallback_class(note: &str) -> &'static str {
    StopReasonKind::from_stop_note(note).key()
}

/// A breaker stop with the product-specific fallback category.
pub(crate) fn breaker_stop(note: &str) -> TurnStop {
    TurnStop::breaker(note, stop_fallback_class)
}

/// Classifies the terminal reason for a completed turn.
pub(crate) fn classify_stop(
    breaker_halt: Option<&str>,
    wind_down: bool,
    hit_cap: bool,
) -> Option<TurnStop> {
    TurnStop::classify(breaker_halt, wind_down, hit_cap, stop_fallback_class)
}

/// Classifies a completed turn with the failure classifier's typed cause when
/// the harness carried one through the run. The human-readable stop note stays
/// available for display, but never recovers the class or operation.
pub(crate) fn classify_stop_with_cause(
    breaker_halt: Option<&str>,
    failure_cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
    wind_down: bool,
    hit_cap: bool,
) -> Option<TurnStop> {
    TurnStop::classify_with_cause(
        breaker_halt,
        failure_cause,
        wind_down,
        hit_cap,
        stop_fallback_class,
    )
}

/// Projects an incomplete result reason through the product's stop categories.
pub(crate) fn stop_from_incomplete(reason: &str) -> TurnStop {
    TurnStop::from_incomplete_reason(reason, stop_fallback_class)
}

/// Projects an incomplete result with its typed breaker cause, when available.
pub(crate) fn stop_from_incomplete_with_cause(
    reason: &str,
    failure_cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
) -> TurnStop {
    if let Some(cause) = failure_cause {
        return TurnStop::from_classified_cause(cause);
    }
    stop_from_incomplete(reason)
}

/// Why the breaker stopped the turn, in terms a user can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReasonKind {
    Transient,
    UncertainSideEffect,
    Authentication,
    Permission,
    Policy,
    MissingProgram,
    NotFound,
    InvalidCalls,
    Unavailable,
    SiteRefused,
    RepeatedFailure,
    VariedFailures,
    RepeatingStep,
    OutOfCredits,
    ProviderConfig,
    MissingConnection,
    Generic,
}

impl StopReasonKind {
    /// Every kind, for tests and for a future locale table.
    #[cfg(test)]
    pub(crate) const ALL: [Self; 17] = [
        Self::Transient,
        Self::UncertainSideEffect,
        Self::Authentication,
        Self::Permission,
        Self::Policy,
        Self::MissingProgram,
        Self::NotFound,
        Self::InvalidCalls,
        Self::Unavailable,
        Self::SiteRefused,
        Self::RepeatedFailure,
        Self::VariedFailures,
        Self::RepeatingStep,
        Self::OutOfCredits,
        Self::ProviderConfig,
        Self::MissingConnection,
        Self::Generic,
    ];

    /// Stable localization key.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Transient => "turn_stop.transient",
            Self::UncertainSideEffect => "turn_stop.uncertain_side_effect",
            Self::Authentication => "turn_stop.authentication",
            Self::Permission => "turn_stop.permission",
            Self::Policy => "turn_stop.policy",
            Self::MissingProgram => "turn_stop.missing_program",
            Self::NotFound => "turn_stop.not_found",
            Self::InvalidCalls => "turn_stop.invalid_calls",
            Self::Unavailable => "turn_stop.unavailable",
            Self::SiteRefused => "turn_stop.site_refused",
            Self::RepeatedFailure => "turn_stop.repeated_failure",
            Self::VariedFailures => "turn_stop.varied_failures",
            Self::RepeatingStep => "turn_stop.repeating_step",
            Self::OutOfCredits => "turn_stop.out_of_credits",
            Self::ProviderConfig => "turn_stop.provider_config",
            Self::MissingConnection => "turn_stop.missing_connection",
            Self::Generic => "turn_stop.generic",
        }
    }

    /// English copy completing "I stopped this turn early because …".
    pub(crate) fn copy(self) -> String {
        // Render through TinyAgents' neutral copy table while preserving the
        // OpenHuman settings destinations for the two actionable classes.
        let key = match self {
            Self::Transient => TurnStopCopyKey::Transient,
            Self::UncertainSideEffect => TurnStopCopyKey::UncertainSideEffect,
            Self::Authentication => TurnStopCopyKey::Authentication,
            Self::Permission => TurnStopCopyKey::Permission,
            Self::Policy => TurnStopCopyKey::Policy,
            Self::MissingProgram => TurnStopCopyKey::MissingProgram,
            Self::NotFound => TurnStopCopyKey::NotFound,
            Self::InvalidCalls => TurnStopCopyKey::InvalidCalls,
            Self::Unavailable => TurnStopCopyKey::Unavailable,
            Self::SiteRefused => TurnStopCopyKey::SiteRefused,
            Self::RepeatedFailure => TurnStopCopyKey::RepeatedFailure,
            Self::VariedFailures => TurnStopCopyKey::VariedFailures,
            Self::RepeatingStep => TurnStopCopyKey::RepeatingStep,
            Self::OutOfCredits => TurnStopCopyKey::OutOfCredits,
            Self::ProviderConfig => TurnStopCopyKey::ProviderConfig,
            Self::MissingConnection => TurnStopCopyKey::MissingConnection,
            Self::Generic => TurnStopCopyKey::Generic,
        };
        tinyagents_harness::turn_stop::render_copy(
            &crate::agent::tinyagents::host::OpenHumanCopyProvider,
            &CopyRequest::TurnStop {
                key,
                operation: None,
            },
        )
    }

    /// Read the kind from a breaker stop note. The notes come from a fixed
    /// set of host and harness templates (`ClassifiedFailureTracker`,
    /// `NoProgressTracker`, `RepeatProgressMiddleware`,
    /// `agent::tinyagents::middleware::repeated_failure`), so this matches
    /// their fixed wording, never the tool output they quote.
    pub(crate) fn from_stop_note(note: &str) -> Self {
        let note = note.trim();
        if note.starts_with("I can't continue without your input") {
            return Self::MissingConnection;
        }
        // Only the note's own lead, before any quoted error, is read.
        let lead = note.split("Last error").next().unwrap_or(note);
        let lead = lead.split("Details:").next().unwrap_or(lead);
        if lead.contains("out of inference budget") {
            return Self::OutOfCredits;
        }
        if lead.contains("configured model/provider rejected") {
            return Self::ProviderConfig;
        }
        if let Some(class) = lead
            .split_once("failure class `")
            .and_then(|(_, rest)| rest.split_once('`'))
            .map(|(class, _)| class)
        {
            return Self::from_failure_class(class);
        }
        if lead.contains("blocked by the security policy") {
            Self::Policy
        } else if lead.contains("recoverable-looking tool failures") {
            Self::Transient
        } else if lead.contains("successful") || lead.contains("identical response") {
            Self::RepeatingStep
        } else if lead.contains("tool calls in a row failed") {
            Self::VariedFailures
        } else if lead.contains("retried") && lead.contains("identical arguments") {
            Self::RepeatedFailure
        } else {
            Self::Generic
        }
    }

    /// The kind for a classified-failure class (`failure_policy.rs`).
    fn from_failure_class(class: &str) -> Self {
        match class {
            "transient" => Self::Transient,
            "uncertain_side_effect" => Self::UncertainSideEffect,
            "authentication" => Self::Authentication,
            "permission" => Self::Permission,
            // `policy` is the user's denial; `blocked_by_policy` is the
            // security policy refusing the call (read-only mode, a forbidden
            // path). Both are the user's security settings, and the refusal
            // used to fall through to `Generic` ("my tool calls were not
            // getting anywhere").
            "policy" | "blocked_by_policy" => Self::Policy,
            "missing_program" => Self::MissingProgram,
            "unsupported" | "missing_app" => Self::MissingProgram,
            "not_found" | "missing_window" => Self::NotFound,
            "invalid_calls" => Self::InvalidCalls,
            "validation" | "invalid_arguments" => Self::InvalidCalls,
            "unavailable" | "service_refused" => Self::Unavailable,
            "site_refused" => Self::SiteRefused,
            "repeated_failure" => Self::RepeatedFailure,
            "varied_failures" => Self::VariedFailures,
            "repeating_step" => Self::RepeatingStep,
            "out_of_credits" => Self::OutOfCredits,
            "provider_config" => Self::ProviderConfig,
            "missing_connection" => Self::MissingConnection,
            _ => Self::Generic,
        }
    }
}

/// One short, scrubbed line describing a failure: the first non-empty line
/// (plus the first stderr line of a command exit report), with credentials
/// and personal identifiers removed, truncated to [`ERROR_LINE_CHARS`].
pub(crate) fn short_error_line(content: &str) -> String {
    let first = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let stderr = content
        .split_once("[stderr]\n")
        .and_then(|(_, tail)| tail.lines().map(str::trim).find(|l| !l.is_empty()))
        .filter(|l| *l != first);
    let line = match stderr {
        Some(err) => format!("{first} — {err}"),
        None => first.to_owned(),
    };
    let line = crate::security::scrub::sanitize_text(&line).value;
    let line = tinymemory_api::safety::redact_identifiers(&line);
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= ERROR_LINE_CHARS {
        return line;
    }
    let cut: String = line.chars().take(ERROR_LINE_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// The user-facing reply for a turn the breaker stopped with `stop_note`.
pub(crate) fn render_stop_summary(results: &[CheckpointToolResult], stop_note: &str) -> String {
    render_stop_summary_with_cause(results, stop_note, None)
}

/// Render a stopped-turn summary from its typed cause when the harness has
/// one. The note is only a compatibility source for old, untyped stops.
pub(crate) fn render_stop_summary_with_cause(
    results: &[CheckpointToolResult],
    stop_note: &str,
    failure_cause: Option<&tinyagents_harness::no_progress::ClassifiedStopCause>,
) -> String {
    let kind = failure_cause
        .map(|cause| StopReasonKind::from_failure_class(&cause.class))
        .unwrap_or_else(|| StopReasonKind::from_stop_note(stop_note));
    tracing::debug!(
        reason = kind.key(),
        results = results.len(),
        "[session-host] rendering stopped-turn summary"
    );
    let groups = tinyagents_harness::turn_stop::group_stop_results(results, |content| {
        short_error_line(content)
    });
    tinyagents_harness::turn_stop::render_stop_summary_with_copy(
        &kind.copy(),
        &groups,
        &super::product_copy::OPENHUMAN_STOP_COPY,
    )
}

/// Deterministic final fallback using OpenHuman copy and stop classification.
pub(crate) fn deterministic_final_summary(
    results: &[CheckpointToolResult],
    stop_reason: Option<&str>,
) -> String {
    if let Some(reason) = stop_reason {
        return render_stop_summary(results, reason);
    }
    if results.is_empty() {
        return tinyagents_harness::close::final_summary_with_copy(
            &super::product_copy::OPENHUMAN_CLOSE_COPY,
            "",
            false,
        );
    }
    let rendered = super::product_copy::render_close_results(results, 4_000);
    tinyagents_harness::close::final_summary_with_copy(
        &super::product_copy::OPENHUMAN_CLOSE_COPY,
        &rendered,
        true,
    )
}

#[cfg(test)]
#[path = "stop_summary_tests.rs"]
mod tests;
