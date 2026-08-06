//! `DomainEvent` → [`Trigger`] normalization.
//!
//! This is the *only* place that knows the shape of the raw bus events. It
//! maps the four v1 trigger sources into a uniform [`Trigger`], producing a
//! redacted `gate_summary` (what the cheap gate model sees) while retaining
//! the full structured payload in `raw` for promotion synthesis.
//!
//! Events we don't care about return `None`.
//!
//! ## Self-trigger guard
//! Proactive deliveries the orchestrator itself emits flow back as channel
//! events; [`normalize`] drops channel messages whose sender marks them as
//! originating from the subconscious so the orchestrator can't trigger
//! itself into an infinite loop. (The richer guard lands with the
//! user-facing thread in slice 6; the hook is here from the start.)

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::core::event_bus::DomainEvent;
use crate::openhuman::subconscious::ORCHESTRATOR_THREAD_ID;

use super::types::{DedupeKey, Trigger, TriggerPayload, TriggerPriority, TriggerSource};

/// Sender marker used by orchestrator-originated proactive messages so the
/// fan-in can skip them (anti self-trigger). Kept in sync with the
/// `notify_user` handoff added in slice 6.
pub const SUBCONSCIOUS_SENDER_MARKER: &str = "subconscious";

/// Max characters of any content preview shown to the gate model.
const PREVIEW_CHARS: usize = 200;

/// Normalize a raw bus event into a [`Trigger`], or `None` if the event is
/// not a registered trigger source. `now` is epoch seconds (injected for
/// determinism); `new_id` supplies the correlation id (injected so tests can
/// assert on it — production passes a fresh uuid).
pub fn normalize_with_id(event: &DomainEvent, now: f64, new_id: String) -> Option<Trigger> {
    match event {
        DomainEvent::CronJobTriggered {
            job_id,
            job_name,
            job_type,
        } => Some(Trigger {
            id: new_id,
            display_label: format!("cron/{job_name}"),
            payload: TriggerPayload {
                gate_summary: format!(
                    "Heartbeat/cron tick for job '{job_name}' (type: {job_type})."
                ),
                raw: serde_json::json!({
                    "job_id": job_id,
                    "job_name": job_name,
                    "job_type": job_type,
                }),
            },
            priority: TriggerPriority::Low,
            // Each firing is intrinsically unique → encode the time so the
            // dedupe window never collapses legitimate repeat ticks.
            dedupe_key: DedupeKey(format!("cron:{job_id}:{}", millis(now))),
            external_content: false,
            received_at: now,
            source: TriggerSource::Cron {
                job_id: job_id.clone(),
                job_name: job_name.clone(),
            },
        }),

        DomainEvent::ChannelInboundMessage {
            channel,
            message,
            sender,
            thread_ts,
            reply_target,
            ..
        } => {
            // Anti self-trigger: skip messages the orchestrator itself sent.
            if sender
                .as_deref()
                .is_some_and(|s| s == SUBCONSCIOUS_SENDER_MARKER)
            {
                return None;
            }
            let thread_id = thread_ts
                .clone()
                .or_else(|| reply_target.clone())
                .unwrap_or_default();
            let sender_label = sender.as_deref().unwrap_or("unknown");
            Some(Trigger {
                id: new_id,
                display_label: format!("user/{channel}"),
                payload: TriggerPayload {
                    gate_summary: format!(
                        "User message on '{channel}' from {sender_label}: {}",
                        preview(message)
                    ),
                    raw: serde_json::json!({
                        "channel": channel,
                        "sender": sender,
                        "thread_id": thread_id,
                        "message": message,
                    }),
                },
                priority: TriggerPriority::High,
                dedupe_key: DedupeKey(format!(
                    "user:{channel}:{thread_id}:{sender_label}:{}",
                    short_hash(message)
                )),
                // Inbound channel messages are untrusted third-party content
                // (a co-channel/remote sender could otherwise drive a
                // full-autonomy promoted run). Taint them so the promoted
                // session runs under `SubconsciousTainted` and the approval
                // gate refuses external-effect tools.
                external_content: true,
                received_at: now,
                source: TriggerSource::UserMessage {
                    channel: channel.clone(),
                    sender: sender.clone(),
                    thread_id,
                },
            })
        }

        DomainEvent::ComposioTriggerReceived {
            toolkit,
            trigger,
            metadata_id,
            payload,
            ..
        } => Some(Trigger {
            id: new_id,
            display_label: format!("composio/{toolkit}/{trigger}"),
            payload: TriggerPayload {
                // Redacted: shape only, never the raw third-party body.
                gate_summary: format!(
                    "Composio webhook '{trigger}' from '{toolkit}' ({} payload fields).",
                    payload_field_count(payload)
                ),
                raw: serde_json::json!({
                    "toolkit": toolkit,
                    "trigger": trigger,
                    "metadata_id": metadata_id,
                    "payload": payload,
                }),
            },
            priority: TriggerPriority::Normal,
            // Same metadata_id ⇒ duplicate delivery ⇒ collapse.
            dedupe_key: DedupeKey(format!("composio:{toolkit}:{trigger}:{metadata_id}")),
            // Third-party content → tainted automation origin downstream.
            external_content: true,
            received_at: now,
            source: TriggerSource::ComposioWebhook {
                toolkit: toolkit.clone(),
                trigger: trigger.clone(),
                metadata_id: metadata_id.clone(),
            },
        }),

        // Only sub-agents spawned by the subconscious orchestrator itself feed
        // back into the pipeline — otherwise unrelated chat/workflow sub-agent
        // completions (the whole `agent` domain) would contaminate the reserved
        // thread and trigger spurious follow-up work.
        DomainEvent::SubagentCompleted {
            parent_session,
            task_id,
            agent_id,
            output_chars,
            iterations,
            ..
        } if parent_session == ORCHESTRATOR_THREAD_ID => Some(Trigger {
            id: new_id,
            display_label: format!("subagent/{agent_id}/done"),
            payload: TriggerPayload {
                gate_summary: format!(
                    "Sub-agent '{agent_id}' (task {task_id}) completed in \
                     {iterations} iteration(s), {output_chars} chars of output."
                ),
                raw: serde_json::json!({
                    "task_id": task_id,
                    "agent_id": agent_id,
                    "ok": true,
                    "output_chars": output_chars,
                    "iterations": iterations,
                }),
            },
            priority: TriggerPriority::Normal,
            // task_id is unique per spawn; completion fires once.
            dedupe_key: DedupeKey(format!("subagent:{task_id}:done")),
            // The sub-agent may have processed untrusted content (e.g. a
            // researcher reading a webhook/email). The completion event does
            // not carry the parent's taint, so fail safe: treat conclusions as
            // tainted so a promoted follow-up turn can't launder untrusted
            // output back into trusted (external-effect) tool access.
            external_content: true,
            received_at: now,
            source: TriggerSource::SubagentConclusion {
                task_id: task_id.clone(),
                agent_id: agent_id.clone(),
                ok: true,
            },
        }),

        DomainEvent::SubagentFailed {
            parent_session,
            task_id,
            agent_id,
            error,
            ..
        } if parent_session == ORCHESTRATOR_THREAD_ID => Some(Trigger {
            id: new_id,
            display_label: format!("subagent/{agent_id}/failed"),
            payload: TriggerPayload {
                gate_summary: format!(
                    "Sub-agent '{agent_id}' (task {task_id}) FAILED: {}",
                    preview(error)
                ),
                raw: serde_json::json!({
                    "task_id": task_id,
                    "agent_id": agent_id,
                    "ok": false,
                    "error": error,
                }),
            },
            priority: TriggerPriority::Normal,
            dedupe_key: DedupeKey(format!("subagent:{task_id}:failed")),
            // Fail safe: conclusions are tainted (see SubagentCompleted above).
            external_content: true,
            received_at: now,
            source: TriggerSource::SubagentConclusion {
                task_id: task_id.clone(),
                agent_id: agent_id.clone(),
                ok: false,
            },
        }),

        _ => None,
    }
}

/// Production entry point: normalize with a fresh uuid correlation id.
pub fn normalize(event: &DomainEvent, now: f64) -> Option<Trigger> {
    normalize_with_id(event, now, uuid::Uuid::new_v4().to_string())
}

/// Truncate `text` to a bounded preview on a char boundary, appending `…`
/// when truncated. Newlines collapsed to spaces for single-line summaries.
fn preview(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        flat
    } else {
        let truncated: String = flat.chars().take(PREVIEW_CHARS).collect();
        format!("{truncated}…")
    }
}

/// Stable-within-process short hash for dedupe keys.
fn short_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Epoch seconds → integer milliseconds for unique-per-firing dedupe keys.
fn millis(now: f64) -> i64 {
    (now * 1000.0) as i64
}

/// Count top-level fields in a JSON payload (0 for non-objects).
fn payload_field_count(payload: &serde_json::Value) -> usize {
    payload.as_object().map(|o| o.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_event_maps_to_low_internal_trigger() {
        let ev = DomainEvent::CronJobTriggered {
            job_id: "job-1".into(),
            job_name: "morning brief".into(),
            job_type: "agent".into(),
        };
        let t = normalize_with_id(&ev, 1.5, "id".into()).unwrap();
        assert_eq!(t.priority, TriggerPriority::Low);
        assert!(!t.external_content);
        assert_eq!(t.source.family(), "cron");
        assert_eq!(t.dedupe_key.as_str(), "cron:job-1:1500");
        assert!(matches!(t.source, TriggerSource::Cron { .. }));
    }

    #[test]
    fn cron_firings_at_different_times_are_not_deduped() {
        let ev = DomainEvent::CronJobTriggered {
            job_id: "job-1".into(),
            job_name: "n".into(),
            job_type: "agent".into(),
        };
        let a = normalize_with_id(&ev, 10.0, "a".into()).unwrap();
        let b = normalize_with_id(&ev, 70.0, "b".into()).unwrap();
        assert_ne!(a.dedupe_key, b.dedupe_key);
    }

    #[test]
    fn user_message_maps_to_high_tainted_trigger() {
        let ev = DomainEvent::ChannelInboundMessage {
            event_name: "msg".into(),
            channel: "slack".into(),
            message: "can you summarize the thread?".into(),
            sender: Some("U123".into()),
            reply_target: Some("D456".into()),
            thread_ts: Some("T789".into()),
            raw_data: serde_json::Value::Null,
        };
        let t = normalize_with_id(&ev, 0.0, "id".into()).unwrap();
        assert_eq!(t.priority, TriggerPriority::High);
        assert!(t.external_content, "inbound channel messages are untrusted");
        assert!(t.payload.gate_summary.contains("summarize the thread"));
        match t.source {
            TriggerSource::UserMessage {
                channel, thread_id, ..
            } => {
                assert_eq!(channel, "slack");
                assert_eq!(thread_id, "T789");
            }
            other => panic!("wrong source {other:?}"),
        }
    }

    #[test]
    fn user_message_from_subconscious_is_dropped() {
        let ev = DomainEvent::ChannelInboundMessage {
            event_name: "msg".into(),
            channel: "slack".into(),
            message: "proactive note".into(),
            sender: Some(SUBCONSCIOUS_SENDER_MARKER.into()),
            reply_target: None,
            thread_ts: None,
            raw_data: serde_json::Value::Null,
        };
        assert!(normalize_with_id(&ev, 0.0, "id".into()).is_none());
    }

    #[test]
    fn composio_webhook_is_tainted_and_redacted() {
        let ev = DomainEvent::ComposioTriggerReceived {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
            metadata_id: "evt-99".into(),
            metadata_uuid: "uuid-99".into(),
            payload: serde_json::json!({"subject": "secret", "body": "do not leak"}),
        };
        let t = normalize_with_id(&ev, 0.0, "id".into()).unwrap();
        assert!(t.external_content);
        assert_eq!(t.priority, TriggerPriority::Normal);
        // Redaction: raw body must NOT appear in the gate summary.
        assert!(!t.payload.gate_summary.contains("do not leak"));
        assert!(t.payload.gate_summary.contains("2 payload fields"));
        assert_eq!(
            t.dedupe_key.as_str(),
            "composio:gmail:GMAIL_NEW_GMAIL_MESSAGE:evt-99"
        );
        // But raw payload is retained for promotion synthesis.
        assert_eq!(t.payload.raw["payload"]["body"], "do not leak");
    }

    #[test]
    fn composio_duplicate_metadata_shares_dedupe_key() {
        let mk = |id: &str| DomainEvent::ComposioTriggerReceived {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW".into(),
            metadata_id: id.into(),
            metadata_uuid: "u".into(),
            payload: serde_json::Value::Null,
        };
        let a = normalize_with_id(&mk("same"), 0.0, "a".into()).unwrap();
        let b = normalize_with_id(&mk("same"), 5.0, "b".into()).unwrap();
        assert_eq!(a.dedupe_key, b.dedupe_key);
    }

    #[test]
    fn subagent_completed_maps_to_ok_conclusion() {
        let ev = DomainEvent::SubagentCompleted {
            parent_session: ORCHESTRATOR_THREAD_ID.into(),
            task_id: "task-7".into(),
            agent_id: "researcher".into(),
            elapsed_ms: 1000,
            output_chars: 42,
            iterations: 3,
        };
        let t = normalize_with_id(&ev, 0.0, "id".into()).unwrap();
        assert!(t.external_content, "conclusions are tainted (fail-safe)");
        assert_eq!(t.dedupe_key.as_str(), "subagent:task-7:done");
        match t.source {
            TriggerSource::SubagentConclusion { ok, agent_id, .. } => {
                assert!(ok);
                assert_eq!(agent_id, "researcher");
            }
            other => panic!("wrong source {other:?}"),
        }
    }

    #[test]
    fn subagent_failed_maps_to_failed_conclusion() {
        let ev = DomainEvent::SubagentFailed {
            parent_session: ORCHESTRATOR_THREAD_ID.into(),
            task_id: "task-8".into(),
            agent_id: "orchestrator".into(),
            error: "max iterations exceeded".into(),
        };
        let t = normalize_with_id(&ev, 0.0, "id".into()).unwrap();
        assert_eq!(t.dedupe_key.as_str(), "subagent:task-8:failed");
        assert!(t.payload.gate_summary.contains("FAILED"));
        match t.source {
            TriggerSource::SubagentConclusion { ok, .. } => assert!(!ok),
            other => panic!("wrong source {other:?}"),
        }
    }

    #[test]
    fn unrelated_event_returns_none() {
        let ev = DomainEvent::ChannelConnected {
            channel: "slack".into(),
        };
        assert!(normalize_with_id(&ev, 0.0, "id".into()).is_none());
    }

    #[test]
    fn long_message_preview_is_truncated() {
        let long = "word ".repeat(100);
        let ev = DomainEvent::ChannelInboundMessage {
            event_name: "m".into(),
            channel: "c".into(),
            message: long,
            sender: None,
            reply_target: None,
            thread_ts: None,
            raw_data: serde_json::Value::Null,
        };
        let t = normalize_with_id(&ev, 0.0, "id".into()).unwrap();
        assert!(t.payload.gate_summary.contains('…'));
    }
}
