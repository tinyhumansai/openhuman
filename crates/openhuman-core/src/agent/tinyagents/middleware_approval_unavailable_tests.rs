use super::super::approval::approval_resolution_without_gate;
use crate::security::approval::gate::ApprovalGateBootState;

#[test]
fn missing_gate_is_allowed_only_after_explicit_operator_disable() {
    let installed = ApprovalGateBootState {
        installed: true,
        disabled_by_env: false,
        override_ignored: false,
        host: "cli",
    };
    let disabled = ApprovalGateBootState {
        installed: false,
        disabled_by_env: true,
        override_ignored: false,
        host: "cli",
    };

    for state in [None, Some(installed)] {
        assert!(matches!(
            approval_resolution_without_gate(state),
            tinyagents_harness::middleware::ApprovalResolution::Deny { .. }
        ));
    }
    assert!(matches!(
        approval_resolution_without_gate(Some(disabled)),
        tinyagents_harness::middleware::ApprovalResolution::Allow { ticket: None }
    ));
}
