use super::*;
use serde_json::json;

#[test]
fn name_is_correct() {
    assert_eq!(AskClarificationTool::new().name(), "ask_user_clarification");
}

#[test]
fn description_is_non_empty() {
    assert!(!AskClarificationTool::new().description().is_empty());
}

#[test]
fn schema_is_object_type() {
    let schema = AskClarificationTool::new().parameters_schema();
    assert_eq!(schema["type"], "object");
}

#[test]
fn permission_level_is_none() {
    assert_eq!(
        AskClarificationTool::new().permission_level(),
        PermissionLevel::None
    );
}

#[test]
fn default_and_new_are_equivalent() {
    let a = AskClarificationTool::new();
    let b = AskClarificationTool::default();
    assert_eq!(a.name(), b.name());
}

#[tokio::test]
async fn execute_with_question_includes_question_in_output() {
    let tool = AskClarificationTool::new();
    let result = tool
        .execute(json!({ "question": "Which branch should I target?" }))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("Which branch should I target?"));
}

#[tokio::test]
async fn execute_with_options_lists_choices() {
    let tool = AskClarificationTool::new();
    let result = tool
        .execute(json!({
            "question": "Which env?",
            "options": ["staging", "production"]
        }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let out = result.output();
    assert!(out.contains("staging"));
    assert!(out.contains("production"));
}

#[tokio::test]
async fn execute_without_question_uses_fallback() {
    let tool = AskClarificationTool::new();
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("clarify"));
}

/// The output is the message the user reads (the early-exit hook captures it
/// verbatim as the pause question), so it must not carry a machine marker no
/// reader parses.
#[tokio::test]
async fn output_is_the_bare_question_with_no_marker() {
    let tool = AskClarificationTool::new();
    let result = tool
        .execute(json!({ "question": "Which branch should I target?" }))
        .await
        .unwrap();
    assert_eq!(result.output(), "Which branch should I target?");
}

/// A blank question must fall back to the generic prompt, not park the turn on
/// an empty one (#6213 review).
///
/// The schema has no `minLength`, so `{"question":""}` is a legal call. The
/// early-exit hook captures this output verbatim as the pause question, so an
/// empty one reaches the user as a blank assistant reply on the channel path
/// and an empty answer box on a delegated sub-agent's card. The old
/// `[CLARIFICATION NEEDED]` prefix masked this by keeping the string non-empty.
#[tokio::test]
async fn a_blank_question_falls_back_to_the_generic_prompt() {
    let tool = AskClarificationTool::new();
    for blank in ["", "   ", "\n\t "] {
        let result = tool
            .execute(serde_json::json!({ "question": blank }))
            .await
            .expect("execute");
        assert!(
            !result.output().trim().is_empty(),
            "a blank question must never produce an empty pause prompt, got {:?}",
            result.output()
        );
        assert_eq!(
            result.output(),
            "Could you clarify?",
            "a blank question must use the same fallback as a missing one"
        );
    }
}

/// The fallback must not swallow a real question that merely has padding.
#[tokio::test]
async fn a_padded_question_is_trimmed_but_kept() {
    let tool = AskClarificationTool::new();
    let result = tool
        .execute(serde_json::json!({ "question": "  Which three sources?  " }))
        .await
        .expect("execute");
    assert_eq!(result.output(), "Which three sources?");
}
