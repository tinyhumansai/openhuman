use super::*;

#[test]
fn seed_snapshot_starts_idle_with_pending_steps() {
    let snap = seed_snapshot();
    assert_eq!(snap.overall, OverallState::Idle);
    assert!(!snap.steps.is_empty());
    assert!(snap.steps.iter().all(|s| s.state == StepState::Pending));
    assert!(snap.started_at.is_none());
    assert!(snap.finished_at.is_none());
}
