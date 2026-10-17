use std::cell::RefCell;

use super::*;
use crate::agent::session_host::turn_checkpoint::FINAL_ANSWER_INSTRUCTION;

const RECORDS: &str = "\n- `list_directory` — ok\n  > three crates\n";
const CLEAN_REPLY: &str = "The workspace holds three crates; the build is green.";
const FALLBACK: &str = "deterministic fallback";

#[test]
fn classified_halt_returns_partial_work_without_provider_usage() {
    use tinyagents_harness::turn_outcome::ToolCallOutcome;
    let outcome = TinyagentsTurnOutcome {
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 2,
        tool_calls: 2,
        input_tokens: 100,
        output_tokens: 20,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        truncated: false,
        breaker_halt: Some("Stopping after 1 attempt(s): failure class `permission` still blocks operation `search` on `catalog`. Resolve this blocker before retrying.".into()),
        tool_outcomes: vec![
            ToolCallOutcome { call_id: "a".into(), name: "list".into(), arguments: serde_json::json!({}), success: true, content: "three items".into(), duration_ms: 1 },
            ToolCallOutcome { call_id: "b".into(), name: "search".into(), arguments: serde_json::json!({}), success: false, content: "403 Forbidden".into(), duration_ms: 1 },
        ],
        compaction: None,
    };
    let close = classified_halt_close(&outcome).expect("classified halt");
    assert_eq!(close.usage.model_calls, 0);
    assert!(close.output.contains("permission"), "{}", close.output);
    assert!(
        close.output.contains("`list` succeeded"),
        "{}",
        close.output
    );
    assert!(
        !close.output.contains("three items"),
        "no raw tool output: {}",
        close.output
    );
    assert!(close.output.contains("403 Forbidden"), "{}", close.output);
}

/// A candidate carrying a verbatim span of the directive it was just handed —
/// the shape this guard exists for, spliced from the constant so a reword
/// cannot leave this fixture quoting text nobody is given.
fn leaking_reply() -> String {
    let quoted = FINAL_ANSWER_INSTRUCTION
        .split('.')
        .next()
        .expect("the instruction opens with a sentence")
        .trim();
    format!("{quoted}. Anyway: the workspace holds three crates.")
}

/// Drive [`close_with_one_repair`] over a scripted sequence of candidates,
/// returning the shipped message, the prompts each attempt was given, the
/// candidates that reached the verifier, and the usage recorded.
async fn run(candidates: Vec<String>, verdicts: Vec<Option<CloseViolation>>) -> Shipped {
    let instruction = final_answer_instruction(None, false, RECORDS);
    let prompts = RefCell::new(Vec::<String>::new());
    let verified = RefCell::new(Vec::<String>::new());
    let remaining = RefCell::new(candidates);
    let remaining_verdicts = RefCell::new(verdicts);

    let ask = |prompt: String| {
        prompts.borrow_mut().push(prompt);
        let candidate = remaining.borrow_mut().remove(0);
        async move { (candidate, None) }
    };
    let verify = |candidate: String| {
        verified.borrow_mut().push(candidate);
        let violation = remaining_verdicts.borrow_mut().remove(0);
        async move { (violation, None) }
    };

    let mut usage = RepairUsage::for_model("test-model");
    let output = close_with_one_repair(
        instruction.clone(),
        None,
        ask,
        verify,
        || FALLBACK.to_string(),
        &mut usage,
    )
    .await;

    Shipped {
        output,
        instruction,
        prompts: prompts.into_inner(),
        verified: verified.into_inner(),
        model_calls: usage.model_calls,
    }
}

struct Shipped {
    output: String,
    instruction: String,
    prompts: Vec<String>,
    verified: Vec<String>,
    model_calls: usize,
}

/// The rung that keeps detection from costing the user the answer: the leak is
/// caught, one corrective re-ask goes out naming it, and the clean second
/// candidate ships instead of a record dump.
#[tokio::test]
async fn a_leaking_candidate_is_repaired_by_one_re_ask() {
    let shipped = run(vec![leaking_reply(), CLEAN_REPLY.to_string()], vec![None]).await;

    assert_eq!(shipped.output, CLEAN_REPLY);
    assert_eq!(shipped.prompts.len(), 2, "exactly one re-ask");
    assert_eq!(
        shipped.prompts[0], shipped.instruction,
        "the first attempt gets the instruction unchanged"
    );
    assert!(
        shipped.prompts[1].contains("repeated these directions"),
        "the re-ask must name the violation: {}",
        shipped.prompts[1]
    );
    assert!(
        shipped.prompts[1].contains(&shipped.instruction),
        "the re-ask must still carry the records: {}",
        shipped.prompts[1]
    );
    assert_eq!(
        shipped.verified,
        vec![CLEAN_REPLY.to_string()],
        "the deterministic guard runs first, so the leak never reaches the checker"
    );
    assert_eq!(
        shipped.model_calls, 3,
        "two closing calls and the one verification"
    );
}

/// The budget is one. A model that quotes the directive twice is not going to
/// stop on a third ask, so the turn takes the deterministic summary.
#[tokio::test]
async fn a_second_leak_falls_back_to_the_deterministic_summary() {
    let shipped = run(vec![leaking_reply(), leaking_reply()], vec![]).await;

    assert_eq!(shipped.output, FALLBACK);
    assert_eq!(shipped.prompts.len(), 2, "no third attempt");
    assert!(
        shipped.verified.is_empty(),
        "neither leak is worth a verification call: {:?}",
        shipped.verified
    );
    assert_eq!(shipped.model_calls, 2);
}

/// A candidate the guard clears but the checker rejects takes the same single
/// rung — the repair is not reserved for quotations.
#[tokio::test]
async fn a_checker_rejection_also_gets_one_repair() {
    let shipped = run(
        vec!["I'll go and look.".to_string(), CLEAN_REPLY.to_string()],
        vec![Some(CloseViolation::Unverified), None],
    )
    .await;

    assert_eq!(shipped.output, CLEAN_REPLY);
    assert!(
        shipped.prompts[1].contains("did not pass the check"),
        "{}",
        shipped.prompts[1]
    );
    assert_eq!(shipped.verified.len(), 2);
    assert_eq!(
        shipped.model_calls, 4,
        "two closing calls, two verifications"
    );
}

/// An empty candidate is a violation like any other, and is worth the re-ask
/// rather than dropping straight to the fallback.
#[tokio::test]
async fn an_empty_candidate_is_re_asked_before_the_fallback() {
    let shipped = run(vec!["   ".to_string(), CLEAN_REPLY.to_string()], vec![None]).await;

    assert_eq!(shipped.output, CLEAN_REPLY);
    assert!(
        shipped.prompts[1].contains("empty or tried to call a tool"),
        "{}",
        shipped.prompts[1]
    );
    assert_eq!(
        shipped.verified,
        vec![CLEAN_REPLY.to_string()],
        "an empty candidate is not sent to the checker"
    );
}

fn blank_outcome(truncated: bool) -> TinyagentsTurnOutcome {
    TinyagentsTurnOutcome {
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 3,
        tool_calls: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        breaker_halt: None,
        truncated,
        compaction: None,
        tool_outcomes: Vec::new(),
    }
}

/// #6951: a turn that ended because the model ran out of output tokens while
/// reasoning is closed with that cause named, not with "you have finished
/// using tools" — which told the model its unfinished work was done.
#[test]
fn a_truncated_turn_is_closed_with_the_output_budget_named() {
    let truncated = close_instruction(&blank_outcome(true), false, RECORDS);
    assert!(
        truncated.contains("ran out of output tokens"),
        "{truncated}"
    );
    assert!(
        !truncated.contains("You have finished using tools"),
        "{truncated}"
    );
    assert!(truncated.contains("<tool_records>"), "{truncated}");

    let finished = close_instruction(&blank_outcome(false), false, RECORDS);
    assert!(
        finished.contains("You have finished using tools"),
        "{finished}"
    );
}

/// The cap checkpoint still wins when the run also hit its call cap.
#[test]
fn a_capped_truncated_turn_gets_the_cap_checkpoint() {
    let out = close_instruction(&blank_outcome(true), true, RECORDS);
    assert!(!out.contains("ran out of output tokens"), "{out}");
    assert!(out.contains("<tool_records>"), "{out}");
}

/// #7716: a close that never answers ships the fallback once the budget runs
/// out; one that finishes in time, or runs without a deadline, ships its reply.
/// On the paused clock the budget elapses without real time passing.
#[tokio::test(start_paused = true)]
async fn close_budget_bounds_the_close_and_falls_back_on_expiry() {
    use std::time::Duration;
    let fallback = || FALLBACK.to_string();

    let stalled = std::future::pending::<String>();
    let output = within_close_budget(Some(Duration::from_secs(20)), stalled, fallback).await;
    assert_eq!(output, FALLBACK);

    for budget in [Some(Duration::from_secs(60)), None] {
        let reply = async { CLEAN_REPLY.to_string() };
        let output = within_close_budget(budget, reply, fallback).await;
        assert_eq!(output, CLEAN_REPLY, "budget {budget:?}");
    }
}

/// A call that finished before the timeout is still billed: the candidate
/// came back, then its verifier stalled past the budget.
#[tokio::test(start_paused = true)]
async fn timed_out_close_keeps_the_usage_of_finished_calls() {
    let instruction = final_answer_instruction(None, false, RECORDS);
    let ask = |_prompt: String| async { (CLEAN_REPLY.to_string(), None) };
    let verify = |_candidate: String| {
        std::future::pending::<(Option<CloseViolation>, Option<BilledUsage>)>()
    };
    let fallback = || FALLBACK.to_string();
    let mut usage = RepairUsage::for_model("test-model");

    let output = within_close_budget(
        Some(std::time::Duration::from_secs(20)),
        close_with_one_repair(instruction, None, ask, verify, fallback, &mut usage),
        fallback,
    )
    .await;

    assert_eq!(output, FALLBACK);
    assert_eq!(usage.model_calls, 1, "the answered ask is recorded");
}

/// The same for the required-output repair (#7721 review): its call answered,
/// then streaming the correction outlived the turn budget. The caller-owned
/// accumulator still holds the call.
#[tokio::test(start_paused = true)]
async fn timed_out_repair_keeps_the_usage_of_its_answered_call() {
    use crate::agent::progress::AgentProgress;
    let source = crate::agent::tinyagents::TurnModelSource::from_model(std::sync::Arc::new(
        tinyagents_harness::testkit::ScriptedModel::replies(vec!["no block"]),
    ));
    let contract = tinyagents_harness::config::RequiredOutput {
        block_key: "result".into(),
        required_keys: vec!["status".into()],
    };
    // A full channel whose receiver is alive: the correction's send blocks.
    let (progress, _receiver) = tokio::sync::mpsc::channel(1);
    assert!(progress
        .try_send(AgentProgress::TextDelta {
            delta: "earlier".into(),
            iteration: 1,
        })
        .is_ok());
    let mut usage = RepairUsage::for_model("test-model");

    let repaired = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        repair_required_output(
            &source,
            "test-model",
            0.0,
            None,
            &tinytools_agent::dialect::NativeDialect,
            &contract,
            &[],
            "Done.",
            true,
            Some(&progress),
            2,
            &mut usage,
        ),
    )
    .await;

    assert!(repaired.is_err(), "the blocked send outlives the budget");
    assert_eq!(usage.model_calls, 1, "the answered repair call is recorded");
}
