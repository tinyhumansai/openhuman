use super::*;
use crate::agent::bus::{mock_agent_run_turn, AgentTurnResponse};
use crate::agent_registry::agents::BUILTINS;
use crate::agent_registry::host_definition::AgentDefinitionRegistry;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc as StdArc;

fn arm_error_label(err: &ArmError) -> &'static str {
    match err {
        ArmError::Retryable { .. } => "Retryable",
        ArmError::Fatal(_) => "Fatal",
        ArmError::BudgetExhausted(_) => "BudgetExhausted",
        ArmError::SafetyFlagged(_) => "SafetyFlagged",
    }
}

// ── Tiered fallback integration tests ───────────────────────────
//
// These drive `run_triage_with_arms` end-to-end through the agent
// bus, with a stateful stub that decides per-call whether to return
// success, a 429, a 5xx, or a fatal auth error. Each `cloud-then-
// local` test relies on call-ordering: cloud arm is exercised
// first; falling through to local arm uses a different
// `provider_name` we inspect to disambiguate.

fn unused_model_source() -> crate::agent::tinyagents::TurnModelSource {
    let model: StdArc<dyn tinyinference_llm::model::ChatModel<()>> =
        StdArc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    crate::agent::tinyagents::TurnModelSource::from_model(model)
}

fn cloud_arm() -> ResolvedProvider {
    ResolvedProvider {
        turn_model_source: unused_model_source(),
        provider_name: "stub-cloud".to_string(),
        model: "stub-cloud-model".to_string(),
        used_local: false,
    }
}

fn local_arm() -> ResolvedProvider {
    ResolvedProvider {
        turn_model_source: unused_model_source(),
        provider_name: "stub-local".to_string(),
        model: "stub-local-model".to_string(),
        used_local: true,
    }
}

fn envelope() -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-x",
        "uuid-x",
        json!({ "from": "ada@example.com", "subject": "ship it" }),
    )
}

const VALID_JSON_REPLY: &str = "{\"action\":\"acknowledge\",\"reason\":\"all good\"}";

#[path = "evaluator_deferral_tests.rs"]
mod deferral_tests;
#[path = "evaluator_fallback_chain_tests.rs"]
mod fallback_chain_tests;
