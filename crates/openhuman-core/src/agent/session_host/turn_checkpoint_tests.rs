use super::*;

fn result(name: &str, success: bool, content: &str) -> CheckpointToolResult {
    CheckpointToolResult {
        name: name.to_string(),
        success,
        content: content.to_string(),
    }
}

/// The point of the rewrite: a capped turn hands back what it actually
/// gathered, not a list of tool names. Asserting on the names alone (which is
/// all the round24 e2e did) would stay green with the whole body dropped
/// (tinysweeper on #6068).
#[test]
fn the_deterministic_checkpoint_reproduces_each_result_body() {
    let out = build_deterministic_checkpoint(
        &[
            result("list_issues", true, "#41 flaky login\n#57 slow boot"),
            result("fetch_pr", false, "404 not found"),
        ],
        25,
    );

    assert!(out.contains("I reached the tool-call limit for this turn (25 steps)"));
    // Each body is reproduced, blockquoted line by line.
    assert!(out.contains("  > #41 flaky login"), "missing body: {out}");
    assert!(out.contains("  > #57 slow boot"), "missing body: {out}");
    assert!(out.contains("  > 404 not found"), "missing body: {out}");
    // Status is carried per result, so a failure is not read as data.
    assert!(out.contains("`list_issues` — ok"));
    assert!(out.contains("`fetch_pr` — failed"));
    // Nothing was dropped, so nothing is disclosed as dropped.
    assert!(!out.contains("omitted for length"));
}

#[test]
fn checkpoint_records_keep_the_captured_tool_failure_status_and_body() {
    let records = results_from_tool_outcomes(&[
        crate::agent::tinyagents::ToolCallOutcome {
            call_id: "call-ok".into(),
            name: "read_file".into(),
            arguments: serde_json::Value::Null,
            success: true,
            content: "configuration found".into(),
            duration_ms: 0,
        },
        crate::agent::tinyagents::ToolCallOutcome {
            call_id: "call-failed".into(),
            name: "apply_patch".into(),
            arguments: serde_json::Value::Null,
            success: false,
            content: "permission denied".into(),
            duration_ms: 0,
        },
    ]);

    let rendered = render_tool_results(&records, CHECKPOINT_TOTAL_CHARS);
    assert!(rendered.contains("`read_file` — ok"));
    assert!(rendered.contains("`apply_patch` — failed"));
    assert!(rendered.contains("permission denied"));
}

/// Over the budget the checkpoint starts later rather than truncating every
/// body, and says how many it skipped — a silent drop reads as "that is all
/// there was".
#[test]
fn results_over_the_budget_are_omitted_with_a_disclosure() {
    // Six bodies of 1k each against a 4k budget: the oldest cannot fit.
    let body = "y".repeat(1_000);
    let results: Vec<CheckpointToolResult> = (0..6)
        .map(|i| result(&format!("tool_{i}"), true, &body))
        .collect();

    let out = build_deterministic_checkpoint(&results, 25);

    assert!(
        out.contains("earlier tool result(s) omitted for length"),
        "an omitted count must be disclosed: {out}"
    );
    // Newest kept, oldest dropped — and the kept ones are in original order.
    assert!(out.contains("`tool_5` — ok"), "newest must survive: {out}");
    assert!(
        !out.contains("`tool_0` — ok"),
        "oldest must be dropped: {out}"
    );
    let five = out.find("`tool_5`").expect("newest rendered");
    let four = out.find("`tool_4`").expect("second-newest rendered");
    assert!(four < five, "kept results must read oldest-first");
}

/// A single oversized payload must not empty the checkpoint: the newest result
/// is shown however long it is, or a capped turn concludes with nothing at all.
#[test]
fn one_oversized_result_is_still_shown() {
    let huge = "z".repeat(CHECKPOINT_TOTAL_CHARS * 3);
    let out = build_deterministic_checkpoint(&[result("dump", true, &huge)], 25);

    assert!(out.contains("`dump` — ok"));
    assert!(out.contains("  > zzz"), "the only result must be rendered");
    assert!(!out.contains("no tools completed yet"));
}

/// The cap can be reached before anything finishes; say so rather than
/// rendering an empty results section.
#[test]
fn no_completed_tools_says_so() {
    let out = build_deterministic_checkpoint(&[], 25);
    assert!(out.contains("(no tools completed yet)"));
    assert!(out.contains("**Next steps:**"));
}

/// The budget must be charged what the checkpoint renders, not the raw body.
/// Every result costs a header line and `"  > "` on each of its lines, so a
/// newline-heavy payload runs well over its body length — and several of them
/// overran the limit while the walk still believed it had room (CodeRabbit on
/// #6068).
#[test]
fn newline_heavy_results_are_charged_what_they_render() {
    // Many one-character lines is the shape that maximises the overhead:
    // the body is ~400 chars and renders to ~1220, since every line pays
    // `"  > "` plus its newline. A gentler shape does not discriminate — 40
    // lines of 20 chars renders 4100 against a 4000 budget, inside this
    // assertion's slack, so the test would have passed against the very code
    // it exists to catch. Verified by replaying both walks over this input.
    let body = (0..200).map(|_| "y").collect::<Vec<_>>().join("\n");
    let results: Vec<CheckpointToolResult> = (0..20)
        .map(|i| result(&format!("list_issues_{i}"), true, &body))
        .collect();

    let out = build_deterministic_checkpoint(&results, 25);

    // The preamble and the "next steps" footer are unconditional; everything
    // beyond them is what the budget governs. Taking the empty-results render
    // as their upper bound keeps this from restating the format string.
    let fixed = build_deterministic_checkpoint(&[], 25).chars().count();
    assert!(
        out.chars().count() <= CHECKPOINT_TOTAL_CHARS + fixed,
        "the rendered checkpoint must respect the budget: {} > {}",
        out.chars().count(),
        CHECKPOINT_TOTAL_CHARS + fixed
    );
    assert!(
        out.contains("earlier tool result(s) omitted for length"),
        "20 results cannot fit, so the omission must be disclosed: {out}"
    );
    // Still useful: the newest result is present with its body.
    assert!(out.contains("`list_issues_19` — ok"));
    assert!(out.contains("  > y\n"), "the newest body must be rendered");
}

const STOP_NOTE: &str = "Stopping: the `install_item` call was retried 3 times with identical \
                         arguments and kept failing. Report this back instead of retrying.";

/// Issue #6278: the fallback used to reduce a failure to the word "failed", and
/// the failure's own message is usually the only explanation of why the
/// request was not done.
#[test]
fn the_final_summary_quotes_each_failure_message() {
    let out = build_deterministic_final_summary(
        &[result(
            "install_item",
            false,
            "no direct download. View it at https://example.test/demo",
        )],
        None,
    );
    assert!(
        out.contains("`install_item` — failed"),
        "status missing: {out}"
    );
    assert!(
        out.contains("  > no direct download. View it at https://example.test/demo"),
        "the failure message must be quoted: {out}"
    );
    assert!(
        !out.contains("Why I stopped"),
        "no halt, no stop section: {out}"
    );
    assert!(!out.contains("tool-call limit"), "not a capped turn: {out}");
}

/// Issue #6279: when the breaker halted the run, the fallback says the turn
/// stopped early and keeps the stop note as a quoted reason, beside the
/// records, instead of standing in for the whole reply.
#[test]
fn the_final_summary_of_a_halted_turn_keeps_the_stop_note_and_the_records() {
    let out = build_deterministic_final_summary(
        &[result("install_item", false, "no direct download")],
        Some(STOP_NOTE),
    );
    assert!(
        out.starts_with("I stopped this turn early"),
        "lead missing: {out}"
    );
    assert!(
        out.contains("**Why I stopped**\n> Stopping:"),
        "stop note must be quoted: {out}"
    );
    assert!(
        out.contains("  > no direct download"),
        "records must follow: {out}"
    );
}

/// The breaker also halts a run whose identical calls keep succeeding
/// (`RepeatProgressMiddleware`). The fallback must not call those calls failed
/// when its own records show them `ok` (Codex review on #6289).
#[test]
fn the_final_summary_of_a_successful_repeat_halt_does_not_claim_failure() {
    let out = build_deterministic_final_summary(
        &[result("list_items", true, "3 items")],
        Some(
            "Stopping: the same successful tool-call batch was issued 3 times in a row with \
             identical arguments and no new information.",
        ),
    );
    assert!(
        !out.to_lowercase().contains("fail"),
        "a halt over successful calls must not be described as failing: {out}"
    );
    assert!(
        out.contains("`list_items` — ok"),
        "records keep their status: {out}"
    );
}

/// The wrap-up is grounded in the records it is handed, and only a halted run
/// passes a stop note.
#[test]
fn the_final_answer_instruction_carries_the_records_and_the_stop_note() {
    let records = render_tool_results(
        &[result("install_item", false, "no direct download")],
        1_000,
    );

    let plain = final_answer_instruction(None, &records);
    assert!(plain.contains("<tool_records>") && plain.contains("  > no direct download"));
    assert!(!plain.contains("<stop_note>"));
    assert!(plain.contains("do not describe steps you are about to take"));

    let halted = final_answer_instruction(Some(STOP_NOTE), &records);
    assert!(halted.contains(&format!("<stop_note>\n{STOP_NOTE}\n</stop_note>")));
    assert!(halted.contains("  > no direct download"));
}

/// The check is read by its conclusion: the last standalone verdict token wins,
/// a word that merely contains one is not a verdict, and no verdict is unclear.
#[test]
fn the_close_verdict_is_the_last_standalone_verdict_token() {
    assert_eq!(parse_close_verdict("ACCEPT"), CloseVerdict::Accept);
    assert_eq!(parse_close_verdict("reject."), CloseVerdict::Reject);
    assert_eq!(
        parse_close_verdict("It could ACCEPT, but rule 1 applies.\nREJECT"),
        CloseVerdict::Reject
    );
    assert_eq!(parse_close_verdict("UNACCEPTABLE"), CloseVerdict::Unclear);
    assert_eq!(parse_close_verdict(""), CloseVerdict::Unclear);
}

/// The check prompt holds the request, the records and the candidate, so the
/// checker can judge the reply against what actually ran.
#[test]
fn the_close_verification_prompt_holds_request_records_and_reply() {
    let prompt = close_verification_prompt(
        "install the demo item",
        "\n- `install_item` — failed\n  > no direct download\n",
        "I'll search the registry.",
    );
    assert!(prompt.contains("<user_request>\ninstall the demo item\n</user_request>"));
    assert!(prompt.contains("  > no direct download"));
    assert!(prompt.contains("<reply>\nI'll search the registry.\n</reply>"));
    assert!(prompt.contains("ACCEPT or REJECT"));
}
