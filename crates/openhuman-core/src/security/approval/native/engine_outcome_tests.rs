use super::*;

#[test]
fn an_aborted_execution_is_recorded_as_abandoned() {
    assert_eq!(
        module_outcome(ExecutionOutcome::Success),
        ModuleOutcome::Succeeded
    );
    assert_eq!(
        module_outcome(ExecutionOutcome::Failure),
        ModuleOutcome::Failed
    );
    // Nothing the host calls aborted proves the tool never ran, so its effects
    // are unknown: abandoned, never cancelled.
    assert_eq!(
        module_outcome(ExecutionOutcome::Aborted),
        ModuleOutcome::Abandoned
    );
}
