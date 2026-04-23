//! Source-agnostic trigger envelope passed into the triage pipeline.
//!
//! [`TriggerEnvelope`] is deliberately generic over where the event
//! came from — composio today, cron and webhook tomorrow — so every
//! caller goes through the same `run_triage` → `apply_decision` path.
//! The [`TriggerSource`] enum carries source-specific fields that the
//! prompt template can format without the triage core needing any
//! composio-aware code paths.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Where the trigger came from, plus source-specific identifiers the
/// triage prompt wants to surface (toolkit/trigger slug, cron job id,
/// webhook tunnel id, etc.).
#[derive(Debug, Clone)]
pub enum TriggerSource {
    /// A Composio webhook event dispatched through the backend's
    /// socket.io bridge. `toolkit` is the slug like `"gmail"`;
    /// `trigger` is the slug like `"GMAIL_NEW_GMAIL_MESSAGE"`.
    Composio { toolkit: String, trigger: String },
    /// A notification captured from an embedded webview integration
    /// (WhatsApp Web, Gmail, Slack, …) via the recipe event pipeline.
    /// `provider` is the slug like `"gmail"`; `account_id` is the
    /// webview account identifier.
    WebviewIntegration {
        provider: String,
        account_id: String,
    },
    /// An incoming webhook request routed through the webhook tunnel system.
    Webhook {
        tunnel_id: String,
        method: String,
        path: String,
    },
    /// A cron job that completed and whose output feeds the triage pipeline.
    Cron { job_id: String, job_name: String },
    /// An external caller (e.g. another service or RPC client) requesting
    /// an agent trigger directly.
    External { caller_id: String, reason: String },
}

impl TriggerSource {
    /// Short slug used in event-bus fields and log prefixes. Stable
    /// across commits so dashboards can rely on it.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Composio { .. } => "composio",
            Self::WebviewIntegration { .. } => "webview",
            Self::Webhook { .. } => "webhook",
            Self::Cron { .. } => "cron",
            Self::External { .. } => "external",
        }
    }
}

/// A fully-hydrated trigger ready to be fed into the triage pipeline.
///
/// Fields are owned because the envelope crosses a `tokio::spawn`
/// boundary in the composio subscriber and the triage pipeline may
/// retain it for the duration of the LLM round-trip + escalation.
#[derive(Debug, Clone)]
pub struct TriggerEnvelope {
    /// Origin + source-specific identifiers.
    pub source: TriggerSource,

    /// Source-specific stable id for this occurrence. For composio
    /// this is the backend `metadata.uuid`; for cron it will be the
    /// job id, etc. Used as the correlation id in published events.
    pub external_id: String,

    /// Human-friendly single-line label used in log prefixes and the
    /// user-message the triage LLM reads, e.g.
    /// `"composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"`.
    pub display_label: String,

    /// Provider-specific raw payload. Commit 1/2 truncate this to
    /// ~8 KB inside the evaluator before it lands in the user message
    /// so a giant Gmail body cannot blow the local-model context
    /// window.
    pub payload: Value,

    /// Wall-clock receipt time — stamped by the caller so the triage
    /// pipeline can report a meaningful `latency_ms` when it
    /// publishes [`crate::core::event_bus::DomainEvent::TriggerEvaluated`].
    pub received_at: DateTime<Utc>,
}

impl TriggerEnvelope {
    /// Build a `TriggerEnvelope` from the fields of a
    /// `DomainEvent::ComposioTriggerReceived`. The caller matches on
    /// the variant and passes the borrowed fields in — we can't
    /// `impl From<&DomainEvent>` directly because the conversion is
    /// only valid for one variant.
    pub fn from_composio(
        toolkit: &str,
        trigger: &str,
        metadata_id: &str,
        metadata_uuid: &str,
        payload: Value,
    ) -> Self {
        // Prefer the UUID as the stable id since composio's
        // `metadata.id` can repeat across retries according to their
        // docs; `metadata.uuid` is the canonical per-occurrence id.
        // Fall back to `metadata.id` only if uuid is missing so we
        // always have *something* to correlate on.
        let external_id = if !metadata_uuid.is_empty() {
            metadata_uuid.to_string()
        } else {
            metadata_id.to_string()
        };
        Self {
            source: TriggerSource::Composio {
                toolkit: toolkit.to_string(),
                trigger: trigger.to_string(),
            },
            external_id,
            display_label: format!("composio/{toolkit}/{trigger}"),
            payload,
            received_at: Utc::now(),
        }
    }

    /// Build a `TriggerEnvelope` from an incoming webhook request.
    ///
    /// `tunnel_id` is used as the correlation id so webhook responses
    /// can be matched back to their trigger envelope.
    pub fn from_webhook(tunnel_id: &str, method: &str, path: &str, payload: Value) -> Self {
        Self {
            source: TriggerSource::Webhook {
                tunnel_id: tunnel_id.to_string(),
                method: method.to_string(),
                path: path.to_string(),
            },
            external_id: tunnel_id.to_string(),
            display_label: format!("webhook/{method}/{path}"),
            payload,
            received_at: Utc::now(),
        }
    }

    /// Build a `TriggerEnvelope` from a completed cron job.
    ///
    /// `job_id` is used as the correlation id; `output` is embedded in
    /// the payload so the triage LLM can see what the job produced.
    pub fn from_cron(job_id: &str, job_name: &str, output: &str) -> Self {
        Self {
            source: TriggerSource::Cron {
                job_id: job_id.to_string(),
                job_name: job_name.to_string(),
            },
            external_id: job_id.to_string(),
            display_label: format!("cron/{job_name}"),
            payload: serde_json::json!({ "output": output }),
            received_at: Utc::now(),
        }
    }

    /// Build a `TriggerEnvelope` from an external caller.
    ///
    /// `caller_id` is used as the correlation id. `reason` is a short
    /// human-readable label explaining what prompted the trigger (e.g.
    /// `"manual_rpc_test"`, `"ci_pipeline"`, …).
    pub fn from_external(caller_id: &str, reason: &str, payload: Value) -> Self {
        Self {
            source: TriggerSource::External {
                caller_id: caller_id.to_string(),
                reason: reason.to_string(),
            },
            external_id: caller_id.to_string(),
            display_label: format!("external/{caller_id}"),
            payload,
            received_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn composio_envelope_builds_expected_label_and_slug() {
        let env = TriggerEnvelope::from_composio(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "trig-1",
            "uuid-1",
            json!({ "from": "a@b.com" }),
        );
        assert_eq!(env.display_label, "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE");
        assert_eq!(env.external_id, "uuid-1");
        assert_eq!(env.source.slug(), "composio");
        match env.source {
            TriggerSource::Composio { toolkit, trigger } => {
                assert_eq!(toolkit, "gmail");
                assert_eq!(trigger, "GMAIL_NEW_GMAIL_MESSAGE");
            }
            _ => panic!("expected Composio variant"),
        }
        assert_eq!(env.payload["from"], "a@b.com");
    }

    #[test]
    fn composio_envelope_falls_back_to_metadata_id_when_uuid_missing() {
        let env = TriggerEnvelope::from_composio(
            "notion",
            "NOTION_PAGE_UPDATED",
            "trig-fallback",
            "",
            json!({}),
        );
        assert_eq!(env.external_id, "trig-fallback");
    }

    #[test]
    fn webhook_envelope_builds_expected_label_and_slug() {
        let env = TriggerEnvelope::from_webhook(
            "tunnel-uuid-1",
            "POST",
            "/hooks/test",
            json!({ "event": "push" }),
        );
        assert_eq!(env.display_label, "webhook/POST//hooks/test");
        assert_eq!(env.external_id, "tunnel-uuid-1");
        assert_eq!(env.source.slug(), "webhook");
        match env.source {
            TriggerSource::Webhook {
                tunnel_id,
                method,
                path,
            } => {
                assert_eq!(tunnel_id, "tunnel-uuid-1");
                assert_eq!(method, "POST");
                assert_eq!(path, "/hooks/test");
            }
            _ => panic!("expected Webhook variant"),
        }
        assert_eq!(env.payload["event"], "push");
    }

    #[test]
    fn cron_envelope_builds_expected_label_and_slug() {
        let env = TriggerEnvelope::from_cron("job-1", "morning_briefing", "Briefing complete");
        assert_eq!(env.display_label, "cron/morning_briefing");
        assert_eq!(env.external_id, "job-1");
        assert_eq!(env.source.slug(), "cron");
        match env.source {
            TriggerSource::Cron { job_id, job_name } => {
                assert_eq!(job_id, "job-1");
                assert_eq!(job_name, "morning_briefing");
            }
            _ => panic!("expected Cron variant"),
        }
        assert_eq!(env.payload["output"], "Briefing complete");
    }

    #[test]
    fn external_envelope_builds_expected_label_and_slug() {
        let env =
            TriggerEnvelope::from_external("caller-abc", "ci_pipeline", json!({ "ref": "main" }));
        assert_eq!(env.display_label, "external/caller-abc");
        assert_eq!(env.external_id, "caller-abc");
        assert_eq!(env.source.slug(), "external");
        match env.source {
            TriggerSource::External { caller_id, reason } => {
                assert_eq!(caller_id, "caller-abc");
                assert_eq!(reason, "ci_pipeline");
            }
            _ => panic!("expected External variant"),
        }
        assert_eq!(env.payload["ref"], "main");
    }
}
