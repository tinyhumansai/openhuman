use super::*;
use crate::security::policy::Refusal;

#[test]
fn a_policy_refusal_is_final_for_the_retry_loop() {
    // TS-E2E-031: these used to reach the retry loop as "[policy-blocked] …",
    // which it retried as if waiting could change the policy's answer.
    for refusal in [
        Refusal::Protected,
        Refusal::ReadOnly,
        Refusal::Hidden,
        Refusal::NotAllowed,
        Refusal::HighRisk,
        Refusal::Category,
        Refusal::LocalOnly,
    ] {
        let output = refusal_output(refusal);
        assert!(
            output.starts_with(POLICY_REFUSAL_PREFIX),
            "{refusal:?} must be final: {output}"
        );
        assert!(output.ends_with(&refusal.to_string()), "{output}");
    }
}

#[test]
fn an_unavailable_module_or_spent_budget_stays_retryable() {
    for refusal in [Refusal::Unavailable, Refusal::RateLimit] {
        let output = refusal_output(refusal);
        assert!(
            !output.starts_with(POLICY_REFUSAL_PREFIX),
            "{refusal:?} may recover and must stay retryable: {output}"
        );
    }
}
