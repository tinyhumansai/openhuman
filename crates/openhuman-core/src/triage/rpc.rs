//! The `agent.triage_evaluate` controller: run the triage pipeline against a
//! synthetic trigger for testing and replay.
//!
//! The wire name stays `agent.triage_evaluate`; `agent_runs::schemas` lists it
//! in the `agent` namespace's catalogue order and calls in here for the schema
//! and handler.

use crate::core::all::ControllerFuture;
use crate::core::runtime::DispatchCtx;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::util::params::{json_output, optional_string, required_string};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

pub(crate) fn triage_evaluate_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent",
        function: "triage_evaluate",
        description: "Run the trigger-triage classifier against a synthetic trigger \
                      payload for testing and replay. Returns the parsed decision \
                      and timing metadata. When dry_run=true the decision is NOT \
                      acted on (no sub-agent dispatch, no events beyond TriggerEvaluated).",
        inputs: vec![
            required_string("source", "Trigger source slug (e.g. 'composio')."),
            optional_string("toolkit", "Toolkit slug (composio-specific)."),
            optional_string("trigger", "Trigger slug (composio-specific)."),
            optional_string("external_id", "Stable per-occurrence id."),
            required_string("display_label", "Human-friendly label."),
            FieldSchema {
                name: "payload",
                ty: TypeSchema::Json,
                comment: "Trigger payload as JSON.",
                required: true,
            },
            FieldSchema {
                name: "dry_run",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "When true, skip apply_decision (default: false).",
                required: false,
            },
        ],
        outputs: vec![json_output("result", "Triage evaluation result.")],
    }
}

#[derive(Debug, Deserialize)]
struct TriageEvaluateParams {
    source: String,
    toolkit: Option<String>,
    trigger: Option<String>,
    external_id: Option<String>,
    display_label: String,
    payload: Value,
    dry_run: Option<bool>,
}

pub(crate) fn handle_triage_evaluate(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<TriageEvaluateParams>(params)?;

        tracing::debug!(
            source = %p.source,
            dry_run = p.dry_run.unwrap_or(false),
            has_external_id = p.external_id.is_some(),
            "[rpc][agent] triage_evaluate received"
        );

        // Build a TriggerEnvelope from the RPC params. Source-specific
        // variants are discriminated by `p.source`.
        let envelope = match p.source.as_str() {
            "composio" => {
                tracing::trace!("[rpc][agent] building composio trigger envelope");
                let toolkit = p.toolkit.as_deref().unwrap_or("unknown");
                let trigger = p.trigger.as_deref().unwrap_or("unknown");
                let eid = p.external_id.as_deref().unwrap_or("rpc");
                crate::triage::TriggerEnvelope::from_composio(
                    toolkit, trigger, "rpc", eid, p.payload,
                )
            }
            "webhook" => {
                tracing::trace!("[rpc][agent] building webhook trigger envelope");
                let tunnel_id = p.external_id.as_deref().unwrap_or("unknown");
                let method = p.toolkit.as_deref().unwrap_or("POST");
                let path = p.trigger.as_deref().unwrap_or("/");
                crate::triage::TriggerEnvelope::from_webhook(tunnel_id, method, path, p.payload)
            }
            "cron" => {
                tracing::trace!("[rpc][agent] building cron trigger envelope");
                let job_id = p.external_id.as_deref().unwrap_or("unknown");
                let job_name = p.display_label.as_str();
                // Preserve the structured payload — extract the output string
                // for the envelope label but keep the full JSON for triage.
                let output = p
                    .payload
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or(job_name);
                crate::triage::TriggerEnvelope::from_cron(job_id, job_name, output)
            }
            "external" => {
                tracing::trace!("[rpc][agent] building external trigger envelope");
                let caller_id = p.external_id.as_deref().unwrap_or("unknown");
                let reason = p.display_label.as_str();
                crate::triage::TriggerEnvelope::from_external(caller_id, reason, p.payload)
            }
            other => {
                tracing::warn!(source = %other, "[rpc][agent] unsupported trigger source");
                return Err(format!(
                    "unsupported trigger source `{other}` — supported: composio, webhook, cron, external"
                ));
            }
        };

        tracing::debug!(
            source = %envelope.source.slug(),
            external_id_len = envelope.external_id.len(),
            "[rpc][agent] running triage pipeline"
        );

        let outcome = crate::triage::run_triage(&ctx, &envelope)
            .await
            .map_err(|e| format!("triage evaluation failed: {e}"))?;

        let dry_run = p.dry_run.unwrap_or(false);
        match outcome {
            crate::triage::TriageOutcome::Decision(run) => {
                if !dry_run {
                    // Locally initiated: this is the desktop's own RPC surface,
                    // so the dispatch keeps the authority its caller already
                    // had. The envelope it builds may name a remote *source*,
                    // but the request itself is local (#5634).
                    let origin = crate::triage::local_trigger_origin();
                    crate::core::runtime::turn_origin::with_origin(
                        origin,
                        crate::triage::apply_decision(&ctx, run.clone(), &envelope),
                    )
                    .await
                    .map_err(|e| format!("apply_decision failed: {e}"))?;
                }

                Ok(serde_json::json!({
                    "decision": run.decision.action.as_str(),
                    "target_agent": run.decision.target_agent,
                    "prompt": run.decision.prompt,
                    "reason": run.decision.reason,
                    "used_local": run.used_local,
                    "latency_ms": run.latency_ms,
                    "resolution_path": run.resolution_path.as_str(),
                    "dry_run": dry_run,
                }))
            }
            crate::triage::TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            } => {
                // Deferred outcome: the chain (cloud → cloud-retry →
                // local) all failed; the caller is expected to
                // re-issue this trigger after `defer_until_ms`. No
                // side effects fire on this path.
                Ok(serde_json::json!({
                    "decision": "deferred",
                    "resolution_path": "deferred",
                    "defer_until_ms": defer_until_ms,
                    "reason": reason,
                    "dry_run": dry_run,
                }))
            }
            crate::triage::TriageOutcome::Terminal { reason } => Ok(serde_json::json!({
                "decision": "terminal",
                "resolution_path": "terminal",
                "reason": reason,
                "dry_run": dry_run,
            })),
        }
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
