//! Structured, actionable messages for policy / permission denials.
//!
//! When the harness blocks a tool call at a policy or permission boundary, the
//! agent must not dead-end with a bare "blocked" line. Each denial is rendered
//! into a structured message — **what** was blocked, **why**, and a concrete
//! **workaround** (how to enable it, or a permitted alternative) — followed by
//! an explicit instruction to relay it to the user rather than halting
//! silently. The rendered string is returned as the (failed) tool result, so it
//! flows back into the turn the same way the unknown-tool corrective error is
//! surfaced to the model (see PR #4360).
//!
//! The relay instruction also states, at the point of denial, that the tool did
//! not run and that no result for it may be reported. Pressure to produce a
//! substantive answer, next to a tool result that carries no output, is an
//! invitation for a weak model to fill the gap: an observed turn had every
//! shell call denied and the agent then reported a fabricated directory listing
//! and `git log` as if they had run. The system prompt's grounding rule did not
//! bite; a contradiction sitting in the tool result itself has a better chance.
//! It is a mitigation, not a guarantee — detecting fabrication needs the
//! tool-call record carried out to the caller.

use crate::openhuman::security::{POLICY_BLOCKED_MARKER, WORKSPACE_MISSING_MARKER};
use crate::openhuman::tools::PermissionLevel;

/// Generic workaround for a raw security-policy / autonomy block. These denials
/// originate deep in the tools / `SecurityPolicy` layer (autonomy tier, command
/// classification, path checks) rather than the pluggable `ToolPolicy`, so there
/// is no single structured reason to lean on — point the agent at the levers that
/// actually unblock the family.
const SECURITY_POLICY_WORKAROUND: &str = "Raise the agent's access tier / autonomy \
    (Settings → Agent access, or the `config.update_autonomy_settings` RPC / \
    `[autonomy]` config) if this action should be allowed; otherwise reach the goal \
    with a permitted (e.g. read-only) alternative, or report that it can't be done \
    here.";

/// A missing workspace cannot be repaired by changing the agent's autonomy.
/// Keep this separate from [`SECURITY_POLICY_WORKAROUND`] so the denial does
/// not send the user toward an unrelated permission change.
const WORKSPACE_MISSING_WORKAROUND: &str = "Create the workspace directory (or configure a valid workspace path) and retry; raising the agent's access tier / autonomy cannot resolve a missing workspace.";

/// The boundary that blocked a tool call, with the context needed to explain it
/// and suggest a way forward.
pub(super) enum PolicyDenial<'a> {
    /// A raw security-policy / autonomy denial emitted by the tools /
    /// `SecurityPolicy` layer, recognised by its [`POLICY_BLOCKED_MARKER`] prefix.
    /// It already states *what* and *why* but carries no workaround or relay
    /// directive; [`render`](Self::render) keeps the marker and reason and appends
    /// the missing guidance. `raw_reason` is the full marker-bearing tool output.
    SecurityPolicyBlocked { tool: &'a str, raw_reason: &'a str },
    /// The session tool policy forbids this tool for the channel's permission
    /// tier (it is not in the allowed set).
    SessionForbidden {
        tool: &'a str,
        required: Option<PermissionLevel>,
        allowed: PermissionLevel,
        channel: &'a str,
    },
    /// The tool is allowed in general, but *this call's* arguments require a
    /// higher permission than the channel grants.
    PermissionTooLow {
        tool: &'a str,
        required: PermissionLevel,
        allowed: PermissionLevel,
        channel: &'a str,
    },
    /// A pluggable `ToolPolicy`
    /// denied the call outright.
    PolicyDenied {
        tool: &'a str,
        policy: &'a str,
        reason: &'a str,
    },
    /// A pluggable `ToolPolicy`
    /// requires an approval handoff this executor cannot complete inline.
    ApprovalRequired {
        tool: &'a str,
        policy: &'a str,
        reason: &'a str,
    },
}

/// Suffix appended to every denial so the agent relays the block instead of
/// silently stopping — *and* does not paper over the absent output by inventing
/// it. The no-output sentence comes first deliberately: the relay directive on
/// its own is pressure to produce a substantive answer, and the prohibition has
/// to bound it before the model reads that pressure.
const RELAY_INSTRUCTION: &str = "The tool did not run and produced no output — \
    do NOT invent or report any result for it. Relay this to the user: explain \
    what was blocked and why, then offer the workaround as the next step. Do \
    not stop silently.";

impl PolicyDenial<'_> {
    /// Render the denial as a structured `Blocked / Reason / Workaround / relay`
    /// message for the model.
    pub(super) fn render(&self) -> String {
        let (blocked, reason, workaround) = match self {
            PolicyDenial::SecurityPolicyBlocked { tool, raw_reason } => {
                // Strip the marker prefix from the reason (it is re-added on the
                // `blocked` line so downstream `contains(POLICY_BLOCKED_MARKER)`
                // checks — classification, the loop-breaker — keep matching).
                let policy_reason = raw_reason
                    .trim()
                    .strip_prefix(POLICY_BLOCKED_MARKER)
                    .unwrap_or(raw_reason)
                    .trim();
                let is_missing_workspace = policy_reason
                    .strip_prefix(WORKSPACE_MISSING_MARKER)
                    .is_some();
                let reason = policy_reason
                    .strip_prefix(WORKSPACE_MISSING_MARKER)
                    .unwrap_or(policy_reason)
                    .trim()
                    .to_string();
                let workaround = if is_missing_workspace {
                    WORKSPACE_MISSING_WORKAROUND
                } else {
                    SECURITY_POLICY_WORKAROUND
                };
                (
                    format!(
                        "{POLICY_BLOCKED_MARKER} Tool '{tool}' was blocked by the security policy"
                    ),
                    reason,
                    workaround.to_string(),
                )
            }
            PolicyDenial::SessionForbidden {
                tool,
                required,
                allowed,
                channel,
            } => {
                let reason = match required {
                    Some(required) => format!(
                        "it requires {required} permission, but the '{channel}' channel only \
                         grants {allowed} access"
                    ),
                    None => format!(
                        "it is not permitted at the '{channel}' channel's {allowed} access tier"
                    ),
                };
                (
                    format!("Tool '{tool}' is blocked by the session tool policy"),
                    reason,
                    raise_tier_workaround(
                        required.map(|p| p.to_string()).as_deref(),
                        *allowed,
                        channel,
                    ),
                )
            }
            PolicyDenial::PermissionTooLow {
                tool,
                required,
                allowed,
                channel,
            } => (
                format!("Tool '{tool}' is blocked by a per-call permission check"),
                format!(
                    "this call needs {required} permission, but the '{channel}' channel only \
                     grants {allowed} access"
                ),
                raise_tier_workaround(Some(&required.to_string()), *allowed, channel),
            ),
            PolicyDenial::PolicyDenied {
                tool,
                policy,
                reason,
            } => (
                format!("Tool '{tool}' was denied by policy '{policy}'"),
                (*reason).to_string(),
                "Address the reason above, or reach the goal with a permitted alternative tool / \
                 path. If this action is genuinely required, ask the user to adjust the policy."
                    .to_string(),
            ),
            PolicyDenial::ApprovalRequired {
                tool,
                policy,
                reason,
            } => (
                format!("Tool '{tool}' requires approval under policy '{policy}'"),
                (*reason).to_string(),
                "Ask the user to approve this action, then retry — or choose an alternative that \
                 does not require approval."
                    .to_string(),
            ),
        };

        format!(
            "Blocked: {blocked}. Reason: {reason}. Workaround: {workaround} {RELAY_INSTRUCTION}"
        )
    }
}

/// Enrich a raw security-policy / autonomy tool result (issue #4094).
///
/// The pluggable-`ToolPolicy` and channel-permission denials are already rendered
/// with a `Blocked / Reason / Workaround / relay` shape by [`ToolPolicyMiddleware`]
/// (PR #4443). But the ~20 `[policy-blocked]` denials emitted deep in
/// `SecurityPolicy` / the tools themselves still return a bare marker line with no
/// workaround and no relay directive, so the agent dead-ends. This wraps such a
/// result into the structured form, appending the workaround + relay while keeping
/// the marker (so classification and the repeated-failure breaker still match).
///
/// Returns `None` — leaving the content untouched — when the result is not a raw
/// policy block, i.e. it lacks the marker, or it already carries a `Workaround:`
/// suffix (an already-structured `ToolPolicyMiddleware` denial that must not be
/// double-wrapped).
pub(super) fn maybe_enrich_policy_block(tool: &str, content: &str) -> Option<String> {
    if !content.contains(POLICY_BLOCKED_MARKER) || content.contains("Workaround:") {
        return None;
    }
    Some(
        PolicyDenial::SecurityPolicyBlocked {
            tool,
            raw_reason: content,
        }
        .render(),
    )
}

/// Workaround shared by the permission-tier denials: raise the channel's
/// agent-access tier, or fall back to a lower-permission tool.
fn raise_tier_workaround(
    required: Option<&str>,
    allowed: PermissionLevel,
    channel: &str,
) -> String {
    match required {
        Some(required) => format!(
            "Raise the '{channel}' channel's agent-access tier to at least {required} \
             (Settings → Agent access, or the `config.update_autonomy_settings` RPC / \
             `[autonomy]` config), or accomplish the goal with a tool that needs only \
             {allowed} access."
        ),
        None => format!(
            "Raise the '{channel}' channel's agent-access tier (Settings → Agent access, or the \
             `config.update_autonomy_settings` RPC / `[autonomy]` config), or accomplish the goal \
             with a tool that needs only {allowed} access."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_forbidden_with_required_lists_reason_and_workaround() {
        let msg = PolicyDenial::SessionForbidden {
            tool: "run_script",
            required: Some(PermissionLevel::Execute),
            allowed: PermissionLevel::ReadOnly,
            channel: "web",
        }
        .render();

        assert!(msg.starts_with("Blocked: Tool 'run_script'"));
        assert!(msg.contains("Reason:"));
        assert!(msg.contains("requires Execute permission"));
        assert!(msg.contains("Workaround:"));
        assert!(msg.contains("agent-access tier"));
        // The relay instruction is what keeps the agent from halting silently.
        assert!(msg.contains("Relay this to the user"));
        // ...and the prohibition is what keeps it from inventing a result.
        assert!(msg.contains("did not run and produced no output"));
    }

    #[test]
    fn session_forbidden_without_required_still_has_workaround() {
        let msg = PolicyDenial::SessionForbidden {
            tool: "run_script",
            required: None,
            allowed: PermissionLevel::ReadOnly,
            channel: "cron",
        }
        .render();

        assert!(msg.contains("not permitted"));
        assert!(msg.contains("Workaround:"));
        assert!(msg.contains("Relay this to the user"));
    }

    #[test]
    fn permission_too_low_names_both_levels() {
        let msg = PolicyDenial::PermissionTooLow {
            tool: "shell",
            required: PermissionLevel::Write,
            allowed: PermissionLevel::ReadOnly,
            channel: "web",
        }
        .render();

        assert!(msg.contains("needs Write permission"));
        assert!(msg.contains("only grants ReadOnly"));
        assert!(msg.contains("Workaround:"));
    }

    #[test]
    fn policy_denied_carries_reason_and_alternative() {
        let msg = PolicyDenial::PolicyDenied {
            tool: "run_script",
            policy: "sandbox",
            reason: "sandbox restriction",
        }
        .render();

        assert!(msg.contains("denied by policy 'sandbox'"));
        assert!(msg.contains("sandbox restriction"));
        assert!(msg.contains("permitted alternative"));
        assert!(msg.contains("Relay this to the user"));
    }

    #[test]
    fn security_policy_block_keeps_marker_and_adds_workaround_and_relay() {
        let raw =
            "[policy-blocked] Security policy: read-only mode — only read commands are allowed";
        let msg = PolicyDenial::SecurityPolicyBlocked {
            tool: "run_command",
            raw_reason: raw,
        }
        .render();

        // The marker survives so classification + the loop-breaker still match.
        assert!(msg.contains(POLICY_BLOCKED_MARKER));
        assert!(msg.starts_with("Blocked:"));
        // The original reason is preserved (without a duplicated marker in it).
        assert!(msg.contains("read-only mode — only read commands are allowed"));
        assert!(msg.contains("Workaround:"));
        assert!(msg.contains("agent-access tier / autonomy") || msg.contains("Agent access"));
        assert!(msg.contains("Relay this to the user"));
    }

    #[test]
    fn missing_workspace_block_suggests_workspace_repair_not_more_autonomy() {
        let raw = format!(
            "{POLICY_BLOCKED_MARKER} {WORKSPACE_MISSING_MARKER} Workspace directory does not exist: C:\\workspace. Nothing can be written until it is created; this is not a path-traversal refusal."
        );
        let msg = PolicyDenial::SecurityPolicyBlocked {
            tool: "write_file",
            raw_reason: &raw,
        }
        .render();

        assert!(msg.contains("Workspace directory does not exist:"));
        assert!(!msg.contains(WORKSPACE_MISSING_MARKER));
        assert!(msg.contains("Create the workspace directory"));
        assert!(msg.contains("cannot resolve a missing workspace"));
        assert!(!msg.contains("Raise the agent's access tier / autonomy"));
        assert!(msg.contains("Relay this to the user"));
    }

    #[test]
    fn workspace_marker_only_classifies_a_prefixed_reason() {
        let raw = format!(
            "{POLICY_BLOCKED_MARKER} unrelated failure mentions {WORKSPACE_MISSING_MARKER}"
        );
        let msg = PolicyDenial::SecurityPolicyBlocked {
            tool: "write_file",
            raw_reason: &raw,
        }
        .render();

        assert!(msg.contains("Raise the agent's access tier / autonomy"));
        assert!(!msg.contains("Create the workspace directory"));
    }

    #[test]
    fn maybe_enrich_only_touches_raw_marker_results() {
        // A raw marker line with no workaround → enriched.
        let raw = "[policy-blocked] Command not allowed by security policy: rm -rf /";
        let enriched = maybe_enrich_policy_block("run_command", raw)
            .expect("a raw policy block should be enriched");
        assert!(enriched.contains("Workaround:"));
        assert!(enriched.contains("Relay this to the user"));
        assert!(enriched.contains(POLICY_BLOCKED_MARKER));

        // An already-structured ToolPolicyMiddleware denial (has "Workaround:") is
        // left alone — no double-wrapping.
        let already = PolicyDenial::PolicyDenied {
            tool: "run_script",
            policy: "sandbox",
            reason: "sandbox restriction",
        }
        .render();
        assert!(maybe_enrich_policy_block("run_script", &already).is_none());

        // A plain non-policy error is untouched.
        assert!(maybe_enrich_policy_block("read_file", "Error: file not found").is_none());
    }

    /// Every denial — whatever the boundary — must say the tool did not run and
    /// forbid reporting a result for it. A denial that only pressed the model to
    /// "not stop silently" was observed being answered with a fabricated
    /// directory listing and `git log` for commands that never executed.
    #[test]
    fn every_denial_forbids_fabricating_a_result() {
        let denials = [
            PolicyDenial::SecurityPolicyBlocked {
                tool: "run_command",
                raw_reason: "[policy-blocked] Security policy: read-only mode",
            },
            PolicyDenial::SessionForbidden {
                tool: "run_script",
                required: Some(PermissionLevel::Execute),
                allowed: PermissionLevel::ReadOnly,
                channel: "web",
            },
            PolicyDenial::SessionForbidden {
                tool: "run_script",
                required: None,
                allowed: PermissionLevel::ReadOnly,
                channel: "cron",
            },
            PolicyDenial::PermissionTooLow {
                tool: "shell",
                required: PermissionLevel::Write,
                allowed: PermissionLevel::ReadOnly,
                channel: "web",
            },
            PolicyDenial::PolicyDenied {
                tool: "run_script",
                policy: "sandbox",
                reason: "sandbox restriction",
            },
            PolicyDenial::ApprovalRequired {
                tool: "send_email",
                policy: "approval_gate",
                reason: "outbound message needs sign-off",
            },
        ];

        for denial in &denials {
            let msg = denial.render();
            assert!(
                msg.contains("The tool did not run and produced no output"),
                "denial must state the tool never ran: {msg}"
            );
            assert!(
                msg.contains("do NOT invent or report any result for it"),
                "denial must forbid inventing a result: {msg}"
            );
            // The prohibition precedes the relay directive so it bounds it.
            let no_output = msg
                .find("The tool did not run")
                .expect("no-output sentence present");
            let relay = msg.find("Relay this to the user").expect("relay present");
            assert!(no_output < relay, "prohibition must come first: {msg}");
        }
    }

    /// The enrichment path for raw `[policy-blocked]` results carries the
    /// prohibition too — those are the shell/command denials that triggered the
    /// fabrication in the first place.
    #[test]
    fn enriched_raw_policy_block_forbids_fabricating_a_result() {
        let enriched = maybe_enrich_policy_block(
            "run_command",
            "[policy-blocked] Command not allowed by security policy: ls -la",
        )
        .expect("a raw policy block should be enriched");

        assert!(enriched.contains("The tool did not run and produced no output"));
        assert!(enriched.contains("do NOT invent or report any result for it"));
    }

    #[test]
    fn approval_required_suggests_approval_then_retry() {
        let msg = PolicyDenial::ApprovalRequired {
            tool: "send_email",
            policy: "approval_gate",
            reason: "outbound message needs sign-off",
        }
        .render();

        assert!(msg.contains("requires approval under policy 'approval_gate'"));
        assert!(msg.contains("approve this action"));
        assert!(msg.contains("Relay this to the user"));
    }
}
