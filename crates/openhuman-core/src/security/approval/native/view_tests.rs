use super::*;
use tinysecurity_bus::{
    ApprovalBinding2, AuditCorrelation2, CallerContext, JudgeOutcome2, JudgeThresholds, Origin,
    SubjectScope,
};

fn row(state: ApprovalLifecycle2) -> ApprovalRow2 {
    ApprovalRow2 {
        request_id: "1".repeat(32),
        version: 1,
        binding: ApprovalBinding2 {
            binding_version: 1,
            scope: SubjectScope {
                user: "u".into(),
                workspace: "w".into(),
                agent: "a".into(),
            },
            policy_ref: "p".repeat(64),
            generation: 1,
            call_sha256: "c".repeat(64),
            content_sha256: "d".repeat(64),
            origin: Origin::Operator,
            caller: CallerContext::default(),
        },
        host_instance: "e".repeat(32),
        nonce: "f".repeat(32),
        tool_name: "gmail_send".into(),
        external_effect: true,
        args_redacted: r#"{"to":"x"}"#.into(),
        summary: "send".into(),
        created_at_ms: 1_700_000_000_000,
        expires_at_ms: 1_700_000_600_000,
        state,
        choice: Some(ApprovalChoice2::Once),
        attribution: Some("operator".into()),
        decision_ticket_sha256: None,
        outcome: None,
        reservation_id: None,
        legacy_history_json: None,
        judge: None,
        audit: AuditCorrelation2 {
            event_id: "a".repeat(32),
            expected_tail: 0,
            generation: 1,
        },
    }
}

fn stored(row: ApprovalRow2) -> StoredRow {
    StoredRow {
        row,
        decided_ms: Some(1_700_000_100_000),
    }
}

#[test]
fn an_approved_row_reads_as_the_users_choice() {
    let approved = entry(&stored(row(ApprovalLifecycle2::Executed)), None).unwrap();
    assert_eq!(approved.decision, ApprovalDecision::ApproveOnce);
    assert!(approved.decided_by.is_none(), "a person decided");
    assert_eq!(approved.args_redacted["to"], "x");
    assert_eq!(approved.tool_name, "gmail_send");
    assert_eq!(approved.decided_at.timestamp_millis(), 1_700_000_100_000);

    let always = entry(
        &stored(row(ApprovalLifecycle2::Approved)),
        Some("approve_always_for_tool"),
    )
    .unwrap();
    assert_eq!(always.decision, ApprovalDecision::ApproveAlwaysForTool);
    // A label that is not an approval never turns an approval into a denial.
    let odd = entry(&stored(row(ApprovalLifecycle2::Approved)), Some("deny")).unwrap();
    assert_eq!(odd.decision, ApprovalDecision::ApproveOnce);
}

#[test]
fn a_denied_expired_or_cancelled_row_reads_as_a_denial() {
    for state in [
        ApprovalLifecycle2::Denied,
        ApprovalLifecycle2::Expired,
        ApprovalLifecycle2::Cancelled,
    ] {
        let mut denied = row(state);
        denied.choice = (state == ApprovalLifecycle2::Denied).then_some(ApprovalChoice2::Deny);
        let shown = entry(&stored(denied), Some("approve_once")).unwrap();
        assert_eq!(shown.decision, ApprovalDecision::Deny, "{state:?}");
    }
}

#[test]
fn an_automatic_approval_names_who_decided() {
    let mut automatic = row(ApprovalLifecycle2::Approved);
    automatic.attribution = Some("auto:tool-grant".into());
    assert_eq!(
        entry(&stored(automatic), None)
            .unwrap()
            .decided_by
            .as_deref(),
        Some("auto:tool-grant")
    );
}

#[test]
fn judge_evidence_is_shown_as_fixed_outcome_and_scores() {
    let mut judged = row(ApprovalLifecycle2::Approved);
    judged.attribution = Some("auto:jev".into());
    judged.judge = Some(JudgeEvidence2 {
        outcome: JudgeOutcome2::Qualified,
        question_ids: ["intent", "reversible", "exfiltrate"].map(str::to_owned),
        scores_millionths: Some([950_000, 900_000, 10_000]),
        thresholds: JudgeThresholds {
            intent: 900,
            reversible: 800,
            exfiltrate: 100,
        },
        configuration_sha256: "0".repeat(64),
        cached: true,
    });
    let shown = entry(&stored(judged), None).unwrap();
    let verdict = shown.judge.unwrap();
    assert_eq!(verdict.outcome, "qualified");
    assert_eq!(verdict.intent_millis, Some(950));
    assert_eq!(verdict.reversible_millis, Some(900));
    assert_eq!(verdict.exfiltrate_millis, Some(10));
    assert!(verdict.cached);
}

#[test]
fn a_row_that_never_left_pending_is_not_history() {
    let pending = StoredRow {
        row: row(ApprovalLifecycle2::Pending),
        decided_ms: None,
    };
    assert!(entry(&pending, None).is_none());
}

#[tokio::test]
async fn an_unstored_request_is_unknown_and_a_stored_one_reports_its_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    assert_eq!(row_decided(&config, "nope").await.unwrap(), None);
    assert!(decided_history(&config, 10).await.unwrap().is_empty());
}

#[test]
fn a_standing_flow_trust_reads_as_the_pre_authorization_it_always_was() {
    let mut trust = row(ApprovalLifecycle2::Approved);
    trust.attribution = Some("flow-trust-grant".into());
    trust.choice = Some(ApprovalChoice2::Flow);
    let shown = entry(&stored(trust), None).unwrap();
    assert_eq!(shown.decision, ApprovalDecision::ApproveAlwaysForFlow);
    assert_eq!(shown.tool_name, "gmail_send");
    assert!(shown
        .action_summary
        .starts_with("Pre-authorized for this flow"));
    assert!(shown.expires_at.is_none() && shown.decided_by.is_none());
}

#[test]
fn a_revoked_flow_trust_is_not_a_decision() {
    let mut trust = row(ApprovalLifecycle2::Cancelled);
    trust.attribution = Some("flow-trust-grant".into());
    assert!(entry(&stored(trust), None).is_none());
}

#[test]
fn an_imported_legacy_request_reads_by_the_state_it_ended_in() {
    for (state, expected) in [
        ("allowed", ApprovalDecision::ApproveOnce),
        ("executed", ApprovalDecision::ApproveOnce),
        ("denied", ApprovalDecision::Deny),
        ("expired", ApprovalDecision::Deny),
    ] {
        let mut imported = row(ApprovalLifecycle2::Denied);
        imported.tool_name = "imported-history".into();
        imported.choice = None;
        imported.legacy_history_json = Some(format!(r#"{{"state":"{state}"}}"#));
        let shown = entry(&stored(imported), None).unwrap();
        assert_eq!(shown.decision, expected, "{state}");
        assert_eq!(shown.tool_name, "earlier approval");
    }
}
