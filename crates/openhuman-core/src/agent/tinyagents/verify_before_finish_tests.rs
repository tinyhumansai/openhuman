use super::*;

use tinyagents_harness::limits::RunLimits;
use tinyagents_harness::middleware::{
    CapturedOutcomes, FinalCallWrapUpMiddleware, OutcomesUnavailable,
};
use tinyagents_harness::runtime::RunPolicy;
use tinyagents_harness::testkit::{FakeTool, ScriptedModel};
use tinyagents_harness::tinyinference_llm::message::Message;
use tinyagents_harness::tinyinference_llm::model::ModelResponse;
use tinyagents_harness::tinyinference_llm::tool::ToolCall;

fn activity(rounds: usize, tools: &[&str]) -> FinishActivity {
    FinishActivity {
        tool_rounds: rounds,
        tools_called: tools.iter().map(|t| (*t).to_string()).collect(),
        tools_executed: Default::default(),
        tools_with_side_effects: Default::default(),
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
    drive_with(rounds, tool, subagent, agent, WrapUp::Absent, 50).await
}

/// No captured outcomes: the wrap-up under test only needs its budget notice.
struct NoOutcomes;

impl CapturedOutcomes for NoOutcomes {
    fn content_for(&self, _: &str) -> Result<Option<String>, OutcomesUnavailable> {
        Ok(None)
    }
}

/// How the wrap-up middleware reaches the check in a driven run.
#[derive(Clone, Copy)]
enum WrapUp {
    /// No wrap-up middleware at all.
    Absent,
    /// Installed with a budget notice, and handed to `install` (the wiring the
    /// harness assembly does with `wrap_up_fired`).
    Linked,
    /// Installed with a budget notice but NOT handed to `install`.
    Unlinked,
}

async fn drive_with(
    rounds: usize,
    tool: &str,
    subagent: bool,
    agent: Option<&str>,
    wrap_up: WrapUp,
    max_model_calls: usize,
) -> (usize, String) {
    let mut responses: Vec<ModelResponse> = (0..rounds)
        .map(|i| tool_round(&format!("c{i}"), tool))
        .collect();
    responses.push(ModelResponse::assistant("draft".to_string()));
    responses.push(ModelResponse::assistant("checked".to_string()));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.with_model(Arc::new(ScriptedModel::new(responses)));
    harness.register_tool(Arc::new(FakeTool::returning("lookup", "ok")));
    harness.register_tool(Arc::new(FakeTool::returning("todo", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(max_model_calls)
            .with_max_tool_calls(50),
        ..RunPolicy::default()
    });
    let wrap_up_mw = (!matches!(wrap_up, WrapUp::Absent)).then(|| {
        Arc::new(
            FinalCallWrapUpMiddleware::new("CONCLUDE", "WRITE", Arc::new(NoOutcomes), 0)
                .with_budget_notice([0.5]),
        )
    });
    if let Some(mw) = &wrap_up_mw {
        harness.push_middleware(mw.clone());
    }
    let linked = wrap_up_mw
        .clone()
        .filter(|_| matches!(wrap_up, WrapUp::Linked));
    install(&mut harness, subagent, agent, &linked, None);
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
    for needle in [
        "original request",
        "literal rule",
        "derived from the request",
        "individual requirements",
        "prohibited case",
        "case in parentheses",
        "exact examples, inputs and conditions",
        "complete outcome",
        "expected errors and side effects",
        "check both readings",
        "equivalent paths or workflows",
        "outcomes, notifications and state changes",
        // A check's own leftovers can fail the request: asked for a single
        // polyglot source file, an agent compiled it to verify the C path and
        // left `cmain` beside it. Asked to replace secrets in a repository's
        // files, another also rewrote its history, and the check that diffed
        // against the original commit found it gone.
        "not as your checks left it",
        "remove what your checks created",
        "compiled binaries",
        "exactly what was asked for, no more",
        "irreversible change the request did not ask for",
        "rewriting history",
        "say so in your answer instead of making it",
        "re-run the affected checks",
        // The verifier of a filter fed adversarial HTML the agent had never
        // tried; a classifier was graded on images it had not seen; a
        // weight-matrix recovery was declared done with a third of the rows
        // checked against an oracle it could query at will.
        "inputs you have not seen",
        "adversarial ones for anything that filters or validates",
        "check every part of the result against it, not a sample",
        // The request's allowed-command list said `:wq`; the agent wrote `wq`
        // and ticked the box. A query had a reference time to meet; the agent
        // measured against the original instead.
        "exact interface the request names",
        "allowed or forbidden elements where a list is given",
        "state the measured value",
        "stop exploring and",
        "Once every check the contract names passes",
        "named test still failing or never run",
        "make the result satisfy both and test each",
        "let the evidence decide in this order",
        "the prose label last",
    ] {
        assert!(check.contains(needle), "check must mention `{needle}`");
    }
    assert!(
        !check.contains("coding task"),
        "the check applies to any task"
    );
}

/// #6990: the installed policy sends the same requirements check with coding
/// and reporting requests, permits corrective tool calls, then accepts the
/// new answer without injecting a second check.
#[tokio::test]
async fn requirements_check_allows_a_fix_for_different_tasks() {
    for task in [
        "Reject variadic defaults with the exact error 'invalid default argument declaration'.",
        "Report all active projects (including paused projects), and exclude archived projects.",
    ] {
        let mut responses: Vec<_> = (0..MIN_TOOL_ROUNDS)
            .map(|i| tool_round(&format!("c{i}"), "lookup"))
            .collect();
        responses.extend([
            ModelResponse::assistant("draft: my checks passed".to_string()),
            tool_round("fix", "lookup"),
            ModelResponse::assistant("fixed and checked against the requirements".to_string()),
            ModelResponse::assistant("must not be reached".to_string()),
        ]);
        let model = Arc::new(ScriptedModel::new(responses));
        let mut harness: AgentHarness<()> = AgentHarness::new();
        harness.with_model(model.clone());
        harness.register_tool(Arc::new(FakeTool::returning("lookup", "ok")));
        harness.with_policy(RunPolicy {
            limits: RunLimits::default().with_max_model_calls(50),
            ..RunPolicy::default()
        });
        install(&mut harness, false, Some("orchestrator"), &None, None);

        let run = harness
            .invoke_default(&(), vec![Message::user(task)])
            .await
            .expect("task run succeeds");
        assert_eq!(
            run.text().as_deref(),
            Some("fixed and checked against the requirements")
        );
        assert_eq!(run.model_calls, MIN_TOOL_ROUNDS + 3);
        let requests = model.requests();
        let verification = &requests[MIN_TOOL_ROUNDS + 1];
        assert_eq!(
            verification.messages.first().map(Message::text).as_deref(),
            Some(task)
        );
        assert_eq!(
            verification.messages.last().map(Message::text),
            Some(check_message())
        );
        assert_eq!(
            run.messages
                .iter()
                .filter(|message| message.text() == check_message())
                .count(),
            1,
            "corrective tool calls must not reset the check"
        );
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

const BUDGET_ROUNDS: usize = 7;
const BUDGET_CALLS: usize = 12;

#[tokio::test]
async fn a_linked_wrap_up_budget_notice_silences_the_check() {
    let rounds = BUDGET_ROUNDS;
    let (checks, text) = drive_with(
        rounds,
        "lookup",
        false,
        Some("orchestrator"),
        WrapUp::Linked,
        BUDGET_CALLS,
    )
    .await;
    assert_eq!(checks, 0, "a budget notice said finish; no re-verify turn");
    assert_eq!(text, "draft");
}

#[tokio::test]
async fn an_unlinked_wrap_up_does_not_silence_the_check() {
    let (checks, _) = drive_with(
        BUDGET_ROUNDS,
        "lookup",
        false,
        Some("orchestrator"),
        WrapUp::Unlinked,
        BUDGET_CALLS,
    )
    .await;
    assert_eq!(
        checks, 1,
        "without the shared Arc the check cannot see the notice"
    );
}
