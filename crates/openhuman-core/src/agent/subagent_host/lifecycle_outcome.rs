//! Translation from product subagent outcomes to neutral orchestration records.

use super::*;

pub(super) const CLASSIFIED_STOP_CAUSE_EXTENSION: &str = "openhuman.classified_stop_cause";

pub(super) fn encode_classified_stop_cause(
    cause: &tinyagents_harness::no_progress::ClassifiedStopCause,
) -> serde_json::Value {
    serde_json::json!({
        "class": cause.class,
        "operation": cause.operation,
    })
}

pub(super) fn decode_classified_stop_cause(
    value: &serde_json::Value,
    summary: &str,
) -> Option<tinyagents_harness::no_progress::ClassifiedStopCause> {
    Some(tinyagents_harness::no_progress::ClassifiedStopCause {
        class: value.get("class")?.as_str()?.to_string(),
        operation: value.get("operation")?.as_str()?.to_string(),
        summary: summary.to_string(),
    })
}

pub(super) fn incomplete_run_status(
    reason: String,
    extensions: &std::collections::BTreeMap<String, serde_json::Value>,
) -> SubagentRunStatus {
    let failure_cause = extensions
        .get(CLASSIFIED_STOP_CAUSE_EXTENSION)
        .or_else(|| extensions.get("failure_cause"))
        .and_then(|value| decode_classified_stop_cause(value, &reason));
    SubagentRunStatus::Incomplete {
        reason,
        failure_cause,
    }
}

pub(super) fn host_outcome_to_neutral(
    outcome: SubagentRunOutcome,
    definition: &crate::agent_registry::host_definition::AgentDefinition,
    options: &SubagentRunOptions,
) -> SubagentOutcome {
    let failure_cause = outcome
        .failure_cause
        .clone()
        .or_else(|| match &outcome.status {
            SubagentRunStatus::Incomplete { failure_cause, .. } => failure_cause.clone(),
            _ => None,
        });
    let usage_cost = outcome.usage.cost();
    let mut extensions = std::collections::BTreeMap::from_iter([
        ("agent_id".into(), serde_json::json!(definition.id)),
        ("iterations".into(), serde_json::json!(outcome.iterations)),
        (
            "skill_filter_override".into(),
            serde_json::json!(options.skill_filter_override),
        ),
        (
            "elapsed_ms".into(),
            serde_json::json!(outcome.elapsed.as_millis() as u64),
        ),
        (
            "elapsed_ns".into(),
            serde_json::json!(outcome.elapsed.as_nanos().min(u64::MAX as u128) as u64),
        ),
        (
            "artifact_paths".into(),
            serde_json::json!(outcome.artifact_paths),
        ),
    ]);
    if let Some(cause) = &failure_cause {
        extensions.insert(
            "failure_cause".into(),
            serde_json::json!({
                "class": cause.class,
                "operation": cause.operation,
            }),
        );
    }
    let status = match outcome.status {
        SubagentRunStatus::Completed => SubagentOutcomeKind::Completed,
        SubagentRunStatus::AwaitingUser {
            question,
            options: answer_options,
            ..
        } => {
            extensions.insert("options".into(), serde_json::json!(answer_options));
            SubagentOutcomeKind::AwaitingInput(SubagentPause {
                reason: question,
                resume: SubagentResume {
                    history: outcome
                        .final_history
                        .iter()
                        .map(crate::agent::message_convert::chat_message_to_message)
                        .collect(),
                    metadata: std::collections::BTreeMap::from_iter([
                        ("agent_id".into(), definition.id.clone()),
                        (
                            "worker_thread_id".into(),
                            options.worker_thread_id.clone().unwrap_or_default(),
                        ),
                        (
                            "skill_filter_override".into(),
                            options.skill_filter_override.clone().unwrap_or_default(),
                        ),
                        (
                            "model_override".into(),
                            options.model_override.clone().unwrap_or_default(),
                        ),
                    ]),
                    ..SubagentResume::default()
                },
            })
        }
        SubagentRunStatus::Incomplete { reason, .. } => {
            SubagentOutcomeKind::Incomplete(SubagentIncomplete::new(reason))
        }
        SubagentRunStatus::Cancelled => SubagentOutcomeKind::Cancelled,
    };
    let mut neutral = SubagentOutcome {
        task_id: outcome.task_id,
        worker_thread_id: options.worker_thread_id.clone(),
        tier: Some(definition.agent_tier.as_str().to_owned()),
        model_override: options.model_override.clone(),
        cost_usd: usage_cost.usd(),
        cost_source: Some(usage_cost.source),
        extensions: std::mem::take(&mut extensions),
        output: outcome.output,
        history: outcome
            .final_history
            .iter()
            .map(crate::agent::message_convert::chat_message_to_message)
            .collect(),
        status,
        usage: UsageTotals {
            calls: outcome.iterations as u64,
            usage: Usage {
                input_tokens: outcome.usage.input_tokens,
                output_tokens: outcome.usage.output_tokens,
                total_tokens: outcome
                    .usage
                    .input_tokens
                    .saturating_add(outcome.usage.output_tokens),
                cache_read_tokens: outcome.usage.cached_input_tokens,
                // An unknown cost crosses as no charge at all, never as $0.
                charged_amount: outcome
                    .usage
                    .cost()
                    .usd()
                    .map(|usd| ChargedAmount::usd_micros((usd * 1_000_000.0).round() as i64)),
                ..Usage::default()
            },
        },
        artifacts: outcome
            .artifact_paths
            .into_iter()
            .map(|id| ArtifactReference {
                id,
                ..ArtifactReference::default()
            })
            .collect(),
        ..SubagentOutcome::cancelled(String::new())
    };
    if let Some(cause) = failure_cause {
        // Keep only the SDK-normalized class and operation; the human-readable
        // summary may contain private tool or URL details.
        neutral.extensions.insert(
            CLASSIFIED_STOP_CAUSE_EXTENSION.into(),
            encode_classified_stop_cause(&cause),
        );
    }
    neutral
}

#[cfg(test)]
#[path = "lifecycle_outcome_tests.rs"]
mod tests;
