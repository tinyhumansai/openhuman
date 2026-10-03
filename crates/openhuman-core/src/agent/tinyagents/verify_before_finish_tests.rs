use super::*;

use tinyagents_harness::limits::RunLimits;
use tinyagents_harness::runtime::RunPolicy;
use tinyagents_harness::testkit::{FakeTool, ScriptedModel};
use tinyagents_harness::tinyinference_llm::message::Message;
use tinyagents_harness::tinyinference_llm::model::ModelResponse;
use tinyagents_harness::tinyinference_llm::tool::ToolCall;

fn activity(rounds: usize, tools: &[&str]) -> FinishActivity {
    FinishActivity {
        tool_rounds: rounds,
        tools_called: tools.iter().map(|t| (*t).to_string()).collect(),
    }
}

fn tool_round(id: &str, name: &str) -> ModelResponse {
    let mut response = ModelResponse::assistant(String::new());
    response.message.content = Vec::new();
    response.message.tool_calls = vec![ToolCall::new(id, name, serde_json::json!({}))];
    response.finish_reason = Some("tool_calls".to_string());
    response
}

/// Drive a harness with `install` applied, `rounds` tool rounds of `tool`,
/// then a draft and a checked answer. Returns how many check turns the run's
/// transcript carries and its final text.
async fn drive(rounds: usize, tool: &str, subagent: bool, agent: Option<&str>) -> (usize, String) {
    let mut responses: Vec<ModelResponse> = (0..rounds)
        .map(|i| tool_round(&format!("c{i}"), tool))
        .collect();
    responses.push(ModelResponse::assistant("draft".to_string()));
    responses.push(ModelResponse::assistant("checked".to_string()));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.register_model("mock", Arc::new(ScriptedModel::new(responses)));
    harness.register_tool(Arc::new(FakeTool::returning("lookup", "ok")));
    harness.register_tool(Arc::new(FakeTool::returning("todo", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(50)
            .with_max_tool_calls(50),
        ..RunPolicy::default()
    });
    install(&mut harness, subagent, agent);
    let run = harness
        .invoke_default(&(), vec![Message::user("do the task")])
        .await
        .expect("run succeeds");
    let checks = run
        .messages
        .iter()
        .filter(|m| matches!(m, Message::User(_)) && m.text().contains(CHECK_MARKER))
        .count();
    (checks, run.text().unwrap_or_default())
}

#[test]
fn applies_to_root_orchestrator_turns_only() {
    assert!(applies(false, Some("orchestrator")));
    assert!(!applies(true, Some("orchestrator")), "sub-agents never");
    assert!(!applies(false, Some("welcome")), "other root agents never");
    assert!(
        !applies(false, None),
        "a turn without an agent identity never"
    );
}

#[test]
fn triggers_on_enough_tool_rounds_or_a_todo_list() {
    assert!(!should_check(&activity(MIN_TOOL_ROUNDS - 1, &["shell"])));
    assert!(should_check(&activity(MIN_TOOL_ROUNDS, &["shell"])));
    assert!(should_check(&activity(1, &[TODO_TOOL])));
    assert!(!should_check(&activity(0, &[])));
}

#[test]
fn check_is_a_harness_instruction_that_names_the_spec_rules() {
    let check = check_message();
    assert!(check.starts_with("<harness_instruction>"));
    assert!(check.contains(CHECK_MARKER));
    for needle in ["original request", "literal rule", "derived from the spec"] {
        assert!(check.contains(needle), "check must mention `{needle}`");
    }
}

#[tokio::test]
async fn installed_root_orchestrator_turn_checks_once_after_five_rounds() {
    let (checks, text) = drive(MIN_TOOL_ROUNDS, "lookup", false, Some("orchestrator")).await;
    assert_eq!(checks, 1);
    assert_eq!(text, "checked");
}

#[tokio::test]
async fn installed_turn_with_a_todo_list_checks_after_one_round() {
    let (checks, text) = drive(1, TODO_TOOL, false, Some("orchestrator")).await;
    assert_eq!(checks, 1);
    assert_eq!(text, "checked");
}

#[tokio::test]
async fn short_turn_is_not_checked() {
    let (checks, text) = drive(2, "lookup", false, Some("orchestrator")).await;
    assert_eq!(checks, 0);
    assert_eq!(text, "draft");
}

#[tokio::test]
async fn subagent_turn_is_not_checked() {
    let (checks, text) = drive(MIN_TOOL_ROUNDS, "lookup", true, Some("orchestrator")).await;
    assert_eq!(checks, 0);
    assert_eq!(text, "draft");
}
