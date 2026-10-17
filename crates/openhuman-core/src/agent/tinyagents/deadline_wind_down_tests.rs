use super::*;
use std::time::{Duration, Instant};
use tinyagents_harness::turn_deadline::TurnDeadline;

fn handle() -> SteeringHandle {
    crate::agent::tinyagents::host::steering::openhuman_steering_handle(
        crate::agent::tinyagents::host::steering::SteeringRunClass::Interactive,
    )
}

#[test]
fn the_default_harness_ceiling_is_clamped_below_the_backstop() {
    let backstop =
        Duration::from_secs(tinyagents_harness::turn_deadline::DEFAULT_TURN_BACKSTOP_SECS);
    let default_ms = tinyagents_harness::turn_policy::DEFAULT_AGENT_TURN_TIMEOUT_SECS * 1_000;
    assert!(
        default_ms > backstop.as_millis() as u64,
        "precondition: the unclamped ceiling is later than the backstop"
    );
    let now = Instant::now();
    let clamped = TurnDeadline::new(now, backstop)
        .clamp_wall_clock_ms(Some(default_ms), now)
        .expect("clamped");
    assert!(clamped < backstop.as_millis() as u64);
}

#[test]
fn a_wind_down_pause_is_recorded_on_the_turn_sidecar() {
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness.with_policy(crate::agent::tinyagents::run_policy_for(200, false));
    let now = Instant::now();
    let mut run_context = OpenHumanRunContext::new();
    run_context.turn_deadline = Some(TurnDeadline::new(
        now - Duration::from_secs(781),
        Duration::from_secs(900),
    ));
    let mw = install(&mut harness, &Some(handle()), &run_context, &None).expect("installed");
    assert!(!run_context.session_sidecar.lock().unwrap().wind_down);
    assert!(mw.maybe_pause("after_tool", Instant::now()));
    assert!(
        run_context.session_sidecar.lock().unwrap().wind_down,
        "the pause must be visible to the driver"
    );
}
