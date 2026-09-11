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
