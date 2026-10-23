//! What the gate tells the engine about who is asking. The verdict for each class
//! is the module's (see `engine_stub.rs` for the scripted copy of it); these tests
//! pin the mapping and the words the agent is told when the engine refuses.

use super::*;

fn trusted(source: TrustedAutomationSource) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: "job-1".into(),
        source,
    }
}

#[test]
fn every_turn_origin_maps_to_one_engine_class() {
    assert_eq!(origin_class(&web_origin()), CallOrigin::Operator);
    assert_eq!(
        origin_class(&trusted(TrustedAutomationSource::Cron)),
        CallOrigin::Cron
    );
    assert_eq!(
        origin_class(&trusted(TrustedAutomationSource::Background)),
        CallOrigin::TrustedAutomation
    );
    assert_eq!(
        origin_class(&trusted(TrustedAutomationSource::Workflow {
            require_approval: false
        })),
        CallOrigin::TrustedAutomation
    );
    // A flow the user forced to be reviewed is a person's call to answer.
    assert_eq!(
        origin_class(&trusted(TrustedAutomationSource::Workflow {
            require_approval: true
        })),
        CallOrigin::Operator
    );
    assert_eq!(
        origin_class(&AgentTurnOrigin::Cli),
        CallOrigin::LocalOperator
    );
    assert_eq!(
        origin_class(&AgentTurnOrigin::DirectChat),
        CallOrigin::LocalOperator
    );
    assert_eq!(
        origin_class(&AgentTurnOrigin::Unknown),
        CallOrigin::Unlabelled
    );
}

#[test]
fn a_refused_scheduled_job_and_a_refused_unlabelled_call_keep_their_own_explanations() {
    let cron = engine_denial(
        &trusted(TrustedAutomationSource::Cron),
        "gmail_send",
        &EngineError::Refused,
    );
    assert!(
        cron.contains("Scheduled agent turns cannot use 'gmail_send'"),
        "{cron}"
    );
    let unlabelled = engine_denial(&AgentTurnOrigin::Unknown, "cron_add", &EngineError::Refused);
    assert!(
        unlabelled.contains("missing its origin label"),
        "{unlabelled}"
    );
    for text in [&cron, &unlabelled] {
        assert!(text.starts_with(POLICY_DENIED_MARKER));
    }
}

#[test]
fn an_engine_that_could_not_answer_is_reported_as_such_for_every_origin() {
    for origin in [
        trusted(TrustedAutomationSource::Cron),
        AgentTurnOrigin::Unknown,
        AgentTurnOrigin::Cli,
    ] {
        let text = engine_denial(
            &origin,
            "shell",
            &EngineError::Unavailable("module is down".into()),
        );
        assert!(text.contains("could not evaluate 'shell'"), "{text}");
        assert!(text.contains("module is down"));
        assert!(!text.contains("Scheduled agent turns"));
    }
}
