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
    let raw = "[policy-blocked] Security policy: read-only mode — only read commands are allowed";
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
