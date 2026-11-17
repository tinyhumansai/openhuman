use super::runtime_session::{begin_turn_resume, OpenHumanSessionState};
use super::TurnOverrides;
use tinyagents_runtime::ResumeMode;

#[test]
fn transcript_suppression_is_applied_before_resume() {
    let mut state = OpenHumanSessionState::default();
    state.pending_turn_overrides = TurnOverrides {
        suppress_transcript_autoload: true,
        suppress_tools: true,
        ..Default::default()
    };
    let mut resume = ResumeMode::LatestForAgent;

    begin_turn_resume(&mut state, &mut resume);

    assert_eq!(resume, ResumeMode::Never);
    assert!(state.pending_turn_overrides == TurnOverrides::default());
    assert!(state.active_turn_overrides.suppress_transcript_autoload);
    assert!(state.active_turn_overrides.suppress_tools);
}
