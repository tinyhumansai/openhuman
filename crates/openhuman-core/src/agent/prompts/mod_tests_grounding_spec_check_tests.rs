use super::*;

#[test]
fn grounding_contract_carries_the_spec_check_rules() {
    // Issue #6952: checks that cannot fail, cleanup that deletes runtime state,
    // and a final state never seen the way the grader sees it.
    let ctx = ctx_with_identity(None);
    let rendered = SystemPromptBuilder::from_final_body("## Custom Agent\n\nBody.".into())
        .build(&ctx)
        .unwrap();
    for clause in [
        "must mirror how the task is specified or graded",
        "Never delete state, data or services the solution needs at runtime",
        "verify the final state the way a fresh consumer would see it",
        "stated constraints, filters and thresholds as items",
    ] {
        assert!(
            rendered.contains(clause),
            "grounding clause missing: {clause}"
        );
    }
}
