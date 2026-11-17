use super::*;
use tinysecurity_bus::{AuditEventReceipt2, AuditHealth2, AuditSinkResponse2};

fn correlation() -> AuditCorrelation2 {
    AuditCorrelation2 {
        event_id: "a".repeat(32),
        expected_tail: 4,
        generation: 2,
    }
}

fn event(kind: HostAuditKind) -> HostAuditEvent {
    HostAuditEvent {
        kind,
        attribution: "tool:shell".into(),
        arguments: serde_json::json!({"command":"ls"}),
    }
}

#[test]
fn host_execution_and_policy_rows_map_to_recordaudit2_contract() {
    let subject = subject("local", "/work", "agent");
    let execution = request(
        &"7".repeat(32),
        subject.clone(),
        event(HostAuditKind::Execution),
        correlation(),
    )
    .unwrap();
    assert_eq!(execution.kind, AuditKind2::Execution);
    assert_eq!(execution.subject, subject);
    assert_eq!(execution.arguments_json, r#"{"command":"ls"}"#);
    assert!(execution.validate().is_ok());

    let policy = request(
        &"7".repeat(32),
        subject,
        event(HostAuditKind::PolicyDecision),
        correlation(),
    )
    .unwrap();
    assert_eq!(policy.kind, AuditKind2::PolicyDecision);
    assert!(policy.validate().is_ok());
}

fn response(committed: bool, sequence: u64, event_id: &str) -> AuditResponse2 {
    AuditResponse2 {
        permitted: true,
        committed,
        health: if committed {
            AuditHealth2::Healthy
        } else {
            AuditHealth2::Degraded
        },
        receipt: committed.then(|| AuditSinkResponse2 {
            scope: subject("local", "/work", "agent"),
            stream_id: "7".repeat(32),
            events: vec![AuditEventReceipt2 {
                sequence,
                event_id: event_id.into(),
            }],
            new_tail: sequence,
        }),
    }
}

#[test]
fn only_an_exact_committed_receipt_counts_as_a_recorded_row() {
    let audit = correlation();
    assert!(committed(&response(true, 5, &audit.event_id), &audit).is_ok());
    assert!(committed(&response(false, 0, ""), &audit).is_err());
    assert!(committed(&response(true, 5, &"c".repeat(32)), &audit).is_err());
    assert!(committed(&response(true, 8, &audit.event_id), &audit).is_err());
    let mut no_receipt = response(true, 5, &audit.event_id);
    no_receipt.receipt = None;
    assert!(committed(&no_receipt, &audit).is_err());
}
