#[test]
fn compacting_neither_flushes_nor_changes_the_phase() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    assert!(!m.observe(&AgentProgress::ContextCompacting));
    assert_eq!(m.snapshot().phase, Some(TurnPhase::Thinking));
}
