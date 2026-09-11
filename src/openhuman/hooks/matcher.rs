//! Matcher semantics — which occurrences of an event reach a given hook.
//!
//! A matcher is one string, and what it is matched *against* depends on the
//! event, exactly as in Cursor: tool events match the tool name, shell events
//! match the command line, file events match the path. The syntax is uniform:
//!
//! * `*` or an absent matcher — everything.
//! * `Shell|Read|Write` — alternation of exact, case-insensitive names.
//! * `MCP:<tool>` — an MCP tool, by its registered name.
//! * anything else — a regular expression, anchored nowhere (so `^rm ` and
//!   `secrets` both work as written).
//!
//! An invalid regular expression does **not** silently match everything. It
//! matches nothing and logs, because a matcher that fails open turns a typo in
//! a deny rule into a rule that fires on every call — the loudest possible
//! failure mode is the safe one here only for allow rules, and the engine
//! cannot tell which kind it is holding.

use super::types::{HookEvent, HookPayload, ToolPayload};

/// The string a matcher is compared against for a given event, or `None` when
/// the event has no meaningful subject (session lifecycle, compaction).
pub fn subject(event: HookEvent, payload: &HookPayload) -> Option<&str> {
    match (event, payload) {
        (
            HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure
            | HookEvent::BeforeMcpExecution
            | HookEvent::AfterMcpExecution,
            HookPayload::Tool(ToolPayload { tool_name, .. }),
        ) => Some(tool_name.as_str()),
        (
            HookEvent::BeforeShellExecution | HookEvent::AfterShellExecution,
            HookPayload::Shell(shell),
        ) => Some(shell.command.as_str()),
        (HookEvent::BeforeReadFile | HookEvent::AfterFileEdit, HookPayload::File(file)) => {
            Some(file.file_path.as_str())
        }
        (HookEvent::BeforeSubmitPrompt, HookPayload::Prompt(prompt)) => {
            Some(prompt.prompt.as_str())
        }
        (HookEvent::SubagentStart | HookEvent::SubagentStop, HookPayload::Subagent(subagent)) => {
            Some(subagent.subagent_type.as_str())
        }
        (HookEvent::AfterAgentResponse | HookEvent::AfterAgentThought, HookPayload::Text(text)) => {
            Some(text.text.as_str())
        }
        _ => None,
    }
}

/// Whether `matcher` selects `subject`.
///
/// An absent matcher selects everything. An event with no subject is selected
/// by an absent or wildcard matcher only — a hook that asks to match `Shell`
/// should not fire on `sessionStart`.
pub fn matches(matcher: Option<&str>, subject: Option<&str>) -> bool {
    let Some(matcher) = matcher.map(str::trim).filter(|m| !m.is_empty()) else {
        return true;
    };
    if matcher == "*" {
        return true;
    }
    let Some(subject) = subject else {
        return false;
    };
    if matcher.contains('|') && !is_regex_syntax(matcher) {
        return matcher
            .split('|')
            .any(|alternative| name_matches(alternative.trim(), subject));
    }
    if !is_regex_syntax(matcher) {
        return name_matches(matcher, subject);
    }
    match regex::Regex::new(matcher) {
        Ok(regex) => regex.is_match(subject),
        Err(error) => {
            log::warn!("[hooks] invalid matcher regex {matcher:?}: {error}; matching nothing");
            false
        }
    }
}

/// Case-insensitive exact comparison, with `MCP:<tool>` reduced to `<tool>`.
fn name_matches(matcher: &str, subject: &str) -> bool {
    let matcher = matcher.strip_prefix("MCP:").unwrap_or(matcher);
    matcher.eq_ignore_ascii_case(subject)
}

/// A matcher made only of identifier characters is treated as a literal name,
/// not a regex — so `Shell` means the tool called `Shell` and nothing else.
fn is_regex_syntax(matcher: &str) -> bool {
    matcher
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':' || c == '|'))
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
