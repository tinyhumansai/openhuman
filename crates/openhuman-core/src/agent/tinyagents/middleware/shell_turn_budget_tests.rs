use super::*;
use serde_json::json;
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::middleware::ToolInvocationIdentity;

const SECS: fn(u64) -> Duration = Duration::from_secs;

#[test]
fn a_timeout_past_the_turn_remainder_is_reduced_to_leave_a_reserve() {
    let clamp = clamp_shell_timeout(Some(660), SECS(260)).expect("660s does not fit in 260s");
    assert_eq!(clamp.requested, Some(660));
    assert_eq!(clamp.clamped_secs, 170);
    assert_eq!(
        clamp.note(),
        "[timeout_secs reduced from 660s to 170s: only 260s of the turn budget remain]"
    );
}

#[test]
fn a_timeout_that_fits_is_left_alone() {
    assert_eq!(clamp_shell_timeout(Some(100), SECS(260)), None);
    assert_eq!(clamp_shell_timeout(Some(170), SECS(260)), None);
}

#[test]
fn an_unbounded_command_gets_the_turn_remainder_as_its_deadline() {
    for requested in [None, Some(0)] {
        let clamp = clamp_shell_timeout(requested, SECS(3_600)).expect("unbounded is clamped");
        assert_eq!(clamp.requested, None);
        assert_eq!(clamp.clamped_secs, 3_510);
        assert_eq!(
            clamp.note(),
            "[timeout_secs set to 3510s: only 3600s of the turn budget remain]"
        );
    }
}

#[test]
fn near_the_deadline_the_command_gets_half_of_what_is_left() {
    // Less than twice the reserve left: the reserve would eat everything, so
    // the command and the answer split what remains.
    assert_eq!(
        clamp_shell_timeout(Some(600), SECS(100))
            .unwrap()
            .clamped_secs,
        50
    );
    assert_eq!(
        clamp_shell_timeout(Some(600), Duration::ZERO)
            .unwrap()
            .clamped_secs,
        1,
        "never zero, which the shell reads as unbounded"
    );
}

fn shell_call(arguments: serde_json::Value) -> ToolCall {
    ToolCall::new("call-1", "shell", arguments)
}

/// A context whose 1 ms budget is already spent, so the clamp fires without
/// depending on scheduling.
fn spent_context() -> RunContext<()> {
    let ctx = RunContext::new(RunConfig::new("r").with_timeout_ms(1), ());
    std::thread::sleep(Duration::from_millis(5));
    ctx
}

async fn before(budget: &Arc<ShellTurnBudget>, ctx: &mut RunContext<()>, call: &mut ToolCall) {
    Middleware::<(), ()>::before_tool(&budget.clamp(), ctx, &(), call)
        .await
        .expect("before_tool succeeds");
}

async fn after(budget: &Arc<ShellTurnBudget>, ctx: &mut RunContext<()>, result: &mut ToolResult) {
    let identity = ToolInvocationIdentity::new("call-1", "shell");
    Middleware::<(), ()>::after_tool(&budget.notes(), ctx, &(), &identity, result)
        .await
        .expect("after_tool succeeds");
}

#[tokio::test]
async fn a_late_shell_call_is_clamped_and_told_why() {
    let budget = ShellTurnBudget::new(None);
    let mut ctx = spent_context();
    let mut call = shell_call(json!({"command": "make", "timeout_secs": 660}));
    before(&budget, &mut ctx, &mut call).await;
    assert_eq!(call.arguments["timeout_secs"], json!(1));
    assert_eq!(call.arguments["command"], json!("make"));

    let mut result = ToolResult::success("built");
    after(&budget, &mut ctx, &mut result).await;
    let out = result.output();
    assert!(out.starts_with("built"), "{out}");
    assert!(
        out.contains("[timeout_secs reduced from 660s to 1s: only 0s of the turn budget remain]"),
        "{out}"
    );

    let mut next = ToolResult::success("again");
    after(&budget, &mut ctx, &mut next).await;
    assert_eq!(next.output(), "again", "a clamp is reported once");
}

#[tokio::test]
async fn an_injected_deadline_is_reported_only_when_it_fires() {
    let budget = ShellTurnBudget::new(None);
    let mut ctx = spent_context();

    let mut call = shell_call(json!({"command": "ls"}));
    before(&budget, &mut ctx, &mut call).await;
    assert_eq!(call.arguments["timeout_secs"], json!(1));
    let mut quick = ToolResult::success("listing");
    after(&budget, &mut ctx, &mut quick).await;
    assert_eq!(quick.output(), "listing");

    let mut call = shell_call(json!({"command": "sleep 100"}));
    before(&budget, &mut ctx, &mut call).await;
    let mut killed = ToolResult::error(
        "Command timed out after 1s and was killed: the shell tool's timeout_secs limit fired.",
    );
    after(&budget, &mut ctx, &mut killed).await;
    assert!(
        killed.output().contains("[timeout_secs set to 1s:"),
        "{}",
        killed.output()
    );
}

#[tokio::test]
async fn other_tools_and_early_calls_are_untouched() {
    let budget = ShellTurnBudget::new(None);
    let mut ctx = spent_context();
    let mut other = ToolCall::new("call-1", "file_read", json!({"path": "a"}));
    before(&budget, &mut ctx, &mut other).await;
    assert_eq!(other.arguments, json!({"path": "a"}));

    let roomy = ShellTurnBudget::new(Some(SECS(3_600)));
    let mut ctx = RunContext::new(RunConfig::new("r"), ());
    let mut call = shell_call(json!({"command": "make", "timeout_secs": 600}));
    before(&roomy, &mut ctx, &mut call).await;
    assert_eq!(call.arguments["timeout_secs"], json!(600));
}

#[tokio::test]
async fn a_turn_without_a_deadline_is_untouched() {
    let budget = ShellTurnBudget::new(None);
    let mut ctx = RunContext::new(RunConfig::new("r"), ());
    let mut call = shell_call(json!({"command": "make"}));
    before(&budget, &mut ctx, &mut call).await;
    assert_eq!(call.arguments, json!({"command": "make"}));
}

#[test]
fn installing_reads_the_turn_ceiling_from_the_harness_policy() {
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    let mut policy = harness.policy().clone();
    policy.limits.max_wall_clock_ms = Some(120_000);
    harness.with_policy(policy);
    let before = harness.middleware().len();

    let shell = install_time_notes(&mut harness);

    assert_eq!(shell.budget, Some(SECS(120)));
    assert_eq!(
        harness.middleware().len(),
        before + 2,
        "the turn clock and the shell notes are both installed"
    );
}
