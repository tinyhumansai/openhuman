use super::*;

#[test]
fn signed_out_projection_preserves_the_session_error_without_flow_prefix() {
    let evaluation = crate::inference::readiness::ReadinessEvaluation {
        status: "signed_out",
        failures: vec![crate::inference::readiness::ReadinessFailure {
            status: "signed_out",
            session_wide: true,
            role: "chat",
            message: "Inference unavailable: you are signed out. Sign in to OpenHuman to run agent nodes.".into(),
            node_ids: vec!["agent-1".into()],
        }],
    };
    assert_eq!(
        format_evaluation(&evaluation).as_deref(),
        Some("Inference unavailable: you are signed out. Sign in to OpenHuman to run agent nodes.")
    );
}

#[test]
fn mixed_failures_keep_per_role_guidance_order_and_failure_count() {
    let evaluation = crate::inference::readiness::ReadinessEvaluation {
        status: "signed_out",
        failures: vec![
            crate::inference::readiness::ReadinessFailure {
                status: "signed_out",
                session_wide: false,
                role: "chat",
                message: "session_expired".into(),
                node_ids: vec!["agent-chat".into()],
            },
            crate::inference::readiness::ReadinessFailure {
                status: "provider_not_configured",
                session_wide: false,
                role: "reasoning",
                message: "API key not configured for provider".into(),
                node_ids: vec!["agent-reasoning".into(), "agent-reasoning-followup".into()],
            },
        ],
    };
    let message = format_evaluation(&evaluation).expect("failures produce a message");
    assert!(message.starts_with("This flow has 2 agent step(s) that need a working AI provider:"));
    let chat = message.find("role `chat`").unwrap();
    let reasoning = message.find("role `reasoning`").unwrap();
    assert!(
        chat < reasoning,
        "roles retain deterministic order: {message}"
    );
    assert!(message.contains("'agent-reasoning', 'agent-reasoning-followup'"));
    assert!(message.contains("Configure your provider API key in OpenHuman Settings > Providers"));
}
