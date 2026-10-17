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

use std::fmt::Write as _;

use super::turn_checkpoint::CheckpointToolResult;

/// Longest error line shown for one tool, in characters.
const ERROR_LINE_CHARS: usize = 120;

/// Most result lines listed; the rest are counted.
const MAX_LISTED: usize = 8;

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
    pub(crate) fn copy(self) -> &'static str {
        match self {
            Self::Transient => "a service I needed kept timing out or was temporarily unavailable",
            Self::UncertainSideEffect => {
                "an action timed out and I could not confirm whether it went through, so I did \
                 not repeat it"
            }
            Self::Authentication => "a service rejected my credentials",
            Self::Permission => "I don't have permission for a step this needs",
            Self::Policy => "a step this needs is blocked by your security settings",
            Self::MissingProgram => {
                "a program or feature this needs is not available on this device"
            }
            Self::NotFound => "something this needs could not be found",
            Self::InvalidCalls => "a tool kept rejecting my requests as invalid",
            Self::Unavailable => "a tool this needs is not available right now",
            Self::SiteRefused => "a website kept refusing my requests",
            Self::RepeatedFailure => "the same step kept failing",
            Self::VariedFailures => "several different tool calls in a row failed",
            Self::RepeatingStep => "I kept repeating the same step without getting anything new",
            Self::OutOfCredits => {
                "the account is out of inference credits; add credits, then ask me to try again"
            }
            Self::ProviderConfig => {
                "the configured model or provider rejected the request; check the model and API \
                 key in Connections"
            }
            Self::MissingConnection => {
                "a service this needs isn't connected; connect it in Connections, then ask me to \
                 try again"
            }
            Self::Generic => "my tool calls were not getting anywhere",
        }
    }

    /// Read the kind from a breaker stop note. The notes come from a fixed
    /// set of host and harness templates (`ClassifiedFailureTracker`,
    /// `NoProgressTracker`, `RepeatProgressMiddleware`,
    /// `inference::failure_copy::halt`), so this matches
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
            "unsupported" | "missing_app" => Self::MissingProgram,
            "not_found" | "missing_window" => Self::NotFound,
            "validation" | "invalid_arguments" => Self::InvalidCalls,
            "unavailable" | "service_refused" => Self::Unavailable,
            "site_refused" => Self::SiteRefused,
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

/// One collapsed line: a tool, whether it succeeded, the error line for a
/// failure, and how many results it stands for.
struct Group {
    name: String,
    success: bool,
    error: String,
    count: usize,
}

fn times(count: usize) -> String {
    match count {
        1 => String::new(),
        n => format!(" {n} times"),
    }
}

/// The user-facing reply for a turn the breaker stopped with `stop_note`.
pub(crate) fn render_stop_summary(results: &[CheckpointToolResult], stop_note: &str) -> String {
    let kind = StopReasonKind::from_stop_note(stop_note);
    tracing::debug!(
        reason = kind.key(),
        results = results.len(),
        "[turn_checkpoint] rendering deterministic stop summary"
    );
    let mut groups: Vec<Group> = Vec::new();
    for result in results {
        let error = if result.success {
            String::new()
        } else {
            short_error_line(&result.content)
        };
        match groups
            .iter_mut()
            .find(|g| g.name == result.name && g.success == result.success && g.error == error)
        {
            Some(group) => group.count += 1,
            None => groups.push(Group {
                name: result.name.clone(),
                success: result.success,
                error,
                count: 1,
            }),
        }
    }

    let mut out = format!("I stopped this turn early because {}.\n", kind.copy());
    if !groups.is_empty() {
        out.push_str("\n**What happened**\n");
        // Show the most recent groups (the immediate blocker is last), in
        // chronological order.
        let shown_from = groups.len().saturating_sub(MAX_LISTED);
        for group in groups.iter().skip(shown_from) {
            let name = crate::util::truncate_with_ellipsis(&group.name, 60);
            let count = times(group.count);
            let _ = if group.success {
                writeln!(out, "- `{name}` succeeded{count}")
            } else if group.error.is_empty() {
                writeln!(out, "- `{name}` failed{count}")
            } else {
                writeln!(out, "- `{name}` failed{count}: {}", group.error)
            };
        }
        let hidden: usize = groups.iter().take(shown_from).map(|g| g.count).sum();
        if hidden > 0 {
            let _ = writeln!(out, "- …and {hidden} more tool call(s)");
        }
    }
    out.push_str("\nTell me how you'd like to proceed, or ask me to try again.");
    out
}

#[cfg(test)]
#[path = "stop_summary_tests.rs"]
mod tests;
