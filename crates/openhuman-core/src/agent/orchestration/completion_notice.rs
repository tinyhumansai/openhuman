//! OpenHuman's wording for background sub-agent completions.
//!
//! The durable queue, dedupe, tombstones and attempt counting live in the
//! harness (`tinyagents_tasks::CompletionRouter`). What stays here is the text
//! the *parent agent* reads: the batched `<background_agent_*>` notice
//! ([`BackgroundCompletionFormatter`], a `CompletionFormatter`) and the
//! `[BACKGROUND_DELIVERY_FAILED]` notice written straight into a thread when a
//! delivery turn can never succeed ([`build_undelivered_notice`]).

use tinyagents_tasks::{CompletionFormatter, CompletionRecord, CompletionStatus};

/// Cap on the delivery-error text stored in the undelivered-results notice. The
/// notice is a permanent thread message, unlike the transient `chat_error`
/// event that already carries the same string, so an unbounded provider error
/// should not be able to dominate the transcript.
const MAX_PERSISTED_ERROR_CHARS: usize = 500;

/// Terminal disposition of a finished background sub-agent. Drives distinct
/// rendering so a failed / awaiting-input async sub-agent surfaces in chat as
/// such instead of being dropped or mistaken for a success (#4896).
///
/// The harness record carries a [`CompletionStatus`]; the host's three-way
/// outcome rides on it as `Success` / `Failed` / `Incomplete` plus the
/// [`AWAITING_INPUT_LABEL`] label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BackgroundAgentOutcome {
    /// Ran to a usable result (or partial progress framed as such).
    #[default]
    Completed,
    /// The child errored before producing a result.
    Failed,
    /// The child paused asking the user a question and was not continued.
    AwaitingInput,
}

impl BackgroundAgentOutcome {
    /// The harness status this outcome is stored as.
    pub(crate) fn status(self) -> CompletionStatus {
        match self {
            Self::Completed => CompletionStatus::Success,
            Self::Failed => CompletionStatus::Failed,
            Self::AwaitingInput => CompletionStatus::Incomplete,
        }
    }

    /// The outcome a stored record renders as. `Incomplete` is the harness's
    /// word for several things (a timeout, an exhausted budget), so a record
    /// counts as awaiting input only when the host labelled it so.
    pub(crate) fn of(record: &CompletionRecord) -> Self {
        match record.status {
            CompletionStatus::Success => Self::Completed,
            CompletionStatus::Incomplete
                if record.label.as_deref() == Some(AWAITING_INPUT_LABEL) =>
            {
                Self::AwaitingInput
            }
            CompletionStatus::Incomplete
            | CompletionStatus::Failed
            | CompletionStatus::Cancelled => Self::Failed,
        }
    }
}

/// The record label the host stamps on an awaiting-input pause.
pub(crate) const AWAITING_INPUT_LABEL: &str = "awaiting_input";

/// Prefix for internal router keys used by independently deliverable pause
/// notices. The encoded task id follows the prefix; a unique suffix allows a
/// resumed task to pause more than once.
pub(crate) const AWAITING_INPUT_RECORD_PREFIX: &str = "\u{1}host-awaiting-input:";

/// Renders a batch of finished background sub-agents as the single
/// system-injected notice the parent agent reviews.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BackgroundCompletionFormatter;

impl CompletionFormatter for BackgroundCompletionFormatter {
    /// Each result is wrapped in a `<background_agent_result id="…">` tag
    /// carrying its sub-agent process id, so the agent can reference / present
    /// them individually. Empty for an empty batch.
    fn format_batch(&self, records: &[CompletionRecord]) -> String {
        if records.is_empty() {
            return String::new();
        }
        let n = records.len();
        let mut out = String::new();
        out.push_str(&format!(
            "[{n} background sub-agent{} finished while you were busy. Review each result \
             below — including any that FAILED or NEED INPUT — and present what is relevant \
             to the user (never silently drop a failure or an awaiting-input pause). Each is \
             tagged with its sub-agent process id.]\n",
            if n == 1 { "" } else { "s" },
        ));
        out.push_str(&render_results(records));
        out
    }
}

/// Defang any sequence in untrusted sub-agent output that could forge or
/// terminate one of the envelope tags below.
///
/// A sub-agent summary is arbitrary text: it can carry tool-fetched web
/// content, file contents, or anything else the child produced. Interpolated
/// raw, a summary containing `</background_agent_result>` closes its own
/// envelope early and everything after it reads as if it came from the host
/// rather than from the child — and a forged *opening* tag invents a result
/// that no sub-agent produced. Both matter more now that this text can be
/// persisted into the thread verbatim when delivery gives up, because a stored
/// message is replayed to every later turn with the authority of the
/// transcript rather than being a one-shot prompt.
///
/// Only the exact markers that could impersonate this envelope are escaped, so
/// ordinary prose and code in a summary survive unchanged.
fn neutralize_envelope_markers(summary: &str) -> String {
    // Escape the bracket rather than prefixing it: a prefixed `\</tag>` still
    // contains the literal marker, so it reads as a real boundary to anything
    // scanning the text. `&lt;` removes the character that makes it a tag while
    // keeping the content legible and recoverable.
    summary
        .replace("</background_agent_", "&lt;/background_agent_")
        .replace("<background_agent_", "&lt;background_agent_")
}

/// Escape a value placed inside a quoted envelope attribute, so an identifier
/// containing a quote or a tag cannot end the opening tag and forge markup.
fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Recover the real task id for display from a pause record's internal router
/// key. The router needs a unique key per pause, while the user-facing result
/// should keep identifying the resumable task by its ordinary id.
fn display_task_id(record: &CompletionRecord) -> String {
    if BackgroundAgentOutcome::of(record) != BackgroundAgentOutcome::AwaitingInput {
        return record.task_id.clone();
    }
    let Some(encoded) = record
        .task_id
        .strip_prefix(AWAITING_INPUT_RECORD_PREFIX)
        .and_then(|rest| rest.split_once(':').map(|(encoded, _)| encoded))
    else {
        return record.task_id.clone();
    };
    let bytes = encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let hex = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(hex, 16).ok()
        })
        .collect::<Option<Vec<_>>>();
    bytes
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| record.task_id.clone())
}

/// Render each result with its outcome-specific tag. Shared by the normal
/// delivery notice and by the undelivered fallback, so a result reads the same
/// either way and a failure is never dressed up as a completion.
fn render_results(records: &[CompletionRecord]) -> String {
    let mut out = String::new();
    for record in records {
        // Distinct tag per terminal outcome so a failure / awaiting-input result
        // is not presented as a normal completion (#4896).
        let (tag, empty_fallback) = match BackgroundAgentOutcome::of(record) {
            BackgroundAgentOutcome::Completed => {
                ("background_agent_result", "(no output reported)")
            }
            BackgroundAgentOutcome::Failed => (
                "background_agent_failure",
                "(failed with no detail reported)",
            ),
            BackgroundAgentOutcome::AwaitingInput => (
                "background_agent_needs_input",
                "(the sub-agent paused awaiting user input)",
            ),
        };
        let text = record.result.text.trim();
        let mut summary = match (text.is_empty(), &record.result.artifact) {
            (false, _) => neutralize_envelope_markers(text),
            // The harness can move the output into an artifact; say so rather
            // than claiming nothing was reported.
            (true, Some(artifact)) => format!(
                "(the output is stored as artifact \"{}\")",
                escape_attribute(&artifact.id)
            ),
            (true, None) => empty_fallback.to_string(),
        };
        if record.result.omitted_chars > 0 {
            summary.push_str(&format!(
                "\n[{} characters of this output were omitted]",
                record.result.omitted_chars
            ));
        }
        out.push_str(&format!(
            "\n<{tag} id=\"{}\" agent=\"{}\">\n{}\n</{tag}>\n",
            escape_attribute(&display_task_id(record)),
            escape_attribute(&record.agent_id),
            summary,
        ));
    }
    out
}

/// Build the notice written **straight into the thread** when the delivery turn
/// has failed too many times to keep retrying.
///
/// Delivery normally runs a system turn so the agent can present a result in
/// context. When that turn cannot succeed, the results still exist and the user
/// is still owed them — so they are persisted verbatim instead, with an
/// `[BACKGROUND_DELIVERY_FAILED]` envelope saying plainly that this is a failed
/// delivery and why. The envelope follows the `[SUBAGENT_FAILED]` precedent
/// (#4896): the user learns the delegated work finished and could not be
/// delivered normally, rather than the result vanishing or arriving as an
/// unexplained raw dump.
pub(crate) fn build_undelivered_notice(
    gave_up: &[CompletionRecord],
    attempts: u32,
    error: &str,
) -> Option<String> {
    if gave_up.is_empty() {
        return None;
    }
    let n = gave_up.len();
    // The error text is already surfaced to the client on every failed delivery
    // (`chat_error` carries it verbatim), so including it here is not a new
    // exposure — but a stored message is permanent where that event is
    // transient, so bound it and defang it like any other untrusted text.
    let error = neutralize_envelope_markers(error);
    let error = if error.chars().count() > MAX_PERSISTED_ERROR_CHARS {
        let truncated: String = error.chars().take(MAX_PERSISTED_ERROR_CHARS).collect();
        format!("{truncated}… (truncated)")
    } else {
        error
    };
    let mut out = format!(
        "[BACKGROUND_DELIVERY_FAILED] {n} background sub-agent result{} finished, but \
         could not be delivered into this conversation after {attempts} attempts. \
         Last error: {error}\n\nThe {} shown verbatim below so nothing is lost — it has \
         not been reviewed or summarised, because the turn that would have done so is \
         the thing that failed.\n",
        if n == 1 { "" } else { "s" },
        if n == 1 { "result is" } else { "results are" },
    );
    out.push_str(&render_results(gave_up));
    Some(out)
}

#[cfg(test)]
#[path = "completion_notice_tests.rs"]
mod tests;
