//! The LLM gate: decides whether a [`Trigger`] is *promoted* into the
//! long-lived orchestrator session or *dropped*.
//!
//! The gate reuses the existing `agent::triage` pipeline wholesale — a
//! cheap, local-first classifier with cloud retry + fallback (see
//! `agent/triage/evaluator.rs`). We do **not** reinvent the model call: we
//! build a [`TriggerEnvelope`] from the redacted trigger summary, run the
//! triage chain, and map its four-way [`TriageAction`] onto our two-way
//! [`GateDecision`]:
//!
//! | triage action | gate decision                    |
//! |---------------|----------------------------------|
//! | `Drop`        | `Drop { acknowledge: false }`    |
//! | `Acknowledge` | `Drop { acknowledge: true }`     |
//! | `React`       | `Promote` (keep trigger priority)|
//! | `Escalate`    | `Promote` (priority ≥ High)      |
//!
//! Crucially the gate model only ever sees the **redacted** `gate_summary`,
//! never the raw third-party body — that stays in `trigger.payload.raw` for
//! promotion synthesis inside the session (slice 4).
//!
//! A per-hour promotion budget caps the always-on loop's spend: when the
//! budget is exhausted, a would-be `Promote` is downgraded to
//! `Drop { acknowledge: true }` so the trigger is still noted but no
//! reasoning-tier session run is spent.

use std::sync::Mutex;

use crate::openhuman::agent::triage::decision::{TriageAction, TriageDecision};
use crate::openhuman::agent::triage::envelope::{TriggerEnvelope, TriggerSource as TriageSource};
use crate::openhuman::agent::triage::evaluator::{run_triage, TriageOutcome};

use super::types::{GateDecision, Trigger, TriggerPriority, TriggerSource};

/// Per-hour cap on promotions (long-lived session runs). Deterministic
/// under test via injected `now` (epoch seconds).
#[derive(Debug)]
pub struct PromotionBudget {
    max_per_hour: u32,
    /// Epoch-second timestamps of recent promotions, pruned to a 1-hour
    /// sliding window on each check.
    recent: Mutex<Vec<f64>>,
}

const HOUR_SECS: f64 = 3600.0;

impl PromotionBudget {
    pub fn new(max_per_hour: u32) -> Self {
        Self {
            max_per_hour,
            recent: Mutex::new(Vec::new()),
        }
    }

    /// Try to consume one promotion slot. Returns `true` if a slot was
    /// available (and records it), `false` if the hourly cap is reached.
    /// `max_per_hour == 0` disables promotion entirely.
    pub fn try_consume(&self, now: f64) -> bool {
        if self.max_per_hour == 0 {
            return false;
        }
        let mut recent = self.recent.lock().expect("budget mutex poisoned");
        recent.retain(|&ts| now - ts < HOUR_SECS);
        if (recent.len() as u32) < self.max_per_hour {
            recent.push(now);
            true
        } else {
            false
        }
    }

    /// Promotions used in the current sliding hour. Diagnostic aid.
    pub fn used(&self, now: f64) -> usize {
        let recent = self.recent.lock().expect("budget mutex poisoned");
        recent.iter().filter(|&&ts| now - ts < HOUR_SECS).count()
    }
}

/// The gate. Holds the promotion budget; the model call is delegated to
/// `agent::triage`.
#[derive(Debug)]
pub struct GatePass {
    budget: PromotionBudget,
}

impl GatePass {
    pub fn new(max_promotions_per_hour: u32) -> Self {
        Self {
            budget: PromotionBudget::new(max_promotions_per_hour),
        }
    }

    /// Evaluate a trigger through the triage LLM and the promotion budget.
    ///
    /// `now` is epoch seconds (injected for deterministic budget accounting).
    /// This is the integration entry point — it performs a network/model
    /// call via `run_triage`; pure mapping is covered by the unit tests on
    /// [`map_triage_to_gate`] / [`apply_budget`].
    pub async fn evaluate(&self, trigger: &Trigger, now: f64) -> GateDecision {
        tracing::debug!(
            source = trigger.source.family(),
            label = %trigger.display_label,
            dedupe_key = trigger.dedupe_key.as_str(),
            external = trigger.external_content,
            "[subconscious_triggers::gate] evaluating trigger"
        );
        let envelope = build_envelope(trigger);
        let decision = match run_triage(&envelope).await {
            Ok(TriageOutcome::Decision(run)) => {
                tracing::debug!(
                    action = run.decision.action.as_str(),
                    label = %trigger.display_label,
                    "[subconscious_triggers::gate] triage decision"
                );
                map_triage_to_gate(&run.decision, trigger)
            }
            Ok(TriageOutcome::Deferred { reason, .. }) => {
                // Couldn't reach a verdict (both arms down / budget / guard).
                // Acknowledge so the trigger isn't silently lost, but spend
                // no session run.
                tracing::debug!(
                    reason = %reason,
                    label = %trigger.display_label,
                    "[subconscious_triggers::gate] triage deferred → drop(ack)"
                );
                GateDecision::Drop {
                    acknowledge: true,
                    reason: format!("triage deferred: {reason}"),
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    label = %trigger.display_label,
                    "[subconscious_triggers::gate] triage error → drop(ack)"
                );
                GateDecision::Drop {
                    acknowledge: true,
                    reason: format!("triage error: {err}"),
                }
            }
        };
        let final_decision = apply_budget(decision, &self.budget, now);
        tracing::debug!(
            decision = final_decision.as_str(),
            label = %trigger.display_label,
            "[subconscious_triggers::gate] gate verdict"
        );
        final_decision
    }
}

/// Downgrade a `Promote` to an acknowledged `Drop` when the hourly
/// promotion budget is exhausted. `Drop`s pass through untouched.
pub fn apply_budget(decision: GateDecision, budget: &PromotionBudget, now: f64) -> GateDecision {
    match decision {
        GateDecision::Promote {
            synthesized_summary,
            priority,
            reason,
        } => {
            if budget.try_consume(now) {
                GateDecision::Promote {
                    synthesized_summary,
                    priority,
                    reason,
                }
            } else {
                GateDecision::Drop {
                    acknowledge: true,
                    reason: format!("promotion budget exhausted (would have promoted: {reason})"),
                }
            }
        }
        drop @ GateDecision::Drop { .. } => drop,
    }
}

/// Pure mapping from a parsed [`TriageDecision`] to a [`GateDecision`].
pub fn map_triage_to_gate(decision: &TriageDecision, trigger: &Trigger) -> GateDecision {
    match decision.action {
        TriageAction::Drop => GateDecision::Drop {
            acknowledge: false,
            reason: decision.reason.clone(),
        },
        TriageAction::Acknowledge => GateDecision::Drop {
            acknowledge: true,
            reason: decision.reason.clone(),
        },
        TriageAction::React => GateDecision::Promote {
            synthesized_summary: synthesize(trigger, decision),
            // A narrow single-step reaction keeps the trigger's own priority.
            priority: trigger.priority,
            reason: decision.reason.clone(),
        },
        TriageAction::Escalate => GateDecision::Promote {
            synthesized_summary: synthesize(trigger, decision),
            // Multi-step work is at least High so it pre-empts Low cron noise.
            priority: trigger.priority.max(TriggerPriority::High),
            reason: decision.reason.clone(),
        },
    }
}

/// Build the user-turn text appended to the long-lived session on promotion.
/// Combines the redacted trigger context with the gate's synthesized
/// instruction (the triage `prompt` field) when present.
fn synthesize(trigger: &Trigger, decision: &TriageDecision) -> String {
    let mut out = format!(
        "[{}] {}",
        trigger.display_label, trigger.payload.gate_summary
    );
    if let Some(prompt) = decision.prompt.as_deref() {
        if !prompt.trim().is_empty() {
            out.push_str("\n\nGate guidance: ");
            out.push_str(prompt.trim());
        }
    }
    out
}

/// Build a triage [`TriggerEnvelope`] from our [`Trigger`].
///
/// The envelope payload carries the **redacted** `gate_summary` — never the
/// raw third-party body — so the cheap gate model can't be used to exfiltrate
/// untrusted content. The full `raw` payload is reserved for the long-lived
/// session at promotion time.
fn build_envelope(trigger: &Trigger) -> TriggerEnvelope {
    let payload = serde_json::json!({
        "summary": trigger.payload.gate_summary,
        "source": trigger.source.family(),
        "priority": trigger.priority.as_str(),
        "external_content": trigger.external_content,
    });

    let source = match &trigger.source {
        TriggerSource::Cron { job_id, job_name } => TriageSource::Cron {
            job_id: job_id.clone(),
            job_name: job_name.clone(),
        },
        TriggerSource::ComposioWebhook {
            toolkit,
            trigger: t,
            ..
        } => TriageSource::Composio {
            toolkit: toolkit.clone(),
            trigger: t.clone(),
        },
        TriggerSource::UserMessage { channel, .. } => TriageSource::External {
            caller_id: format!("user:{channel}"),
            reason: "user_message".to_string(),
        },
        TriggerSource::SubagentConclusion { agent_id, .. } => TriageSource::External {
            caller_id: format!("subagent:{agent_id}"),
            reason: "subagent_conclusion".to_string(),
        },
    };

    // Preserve the original trigger receipt time (epoch seconds) so triage
    // latency/recency reflects when the event arrived, not gate-eval time.
    let received_at = chrono::DateTime::<chrono::Utc>::from_timestamp(
        trigger.received_at.trunc() as i64,
        (trigger.received_at.fract() * 1_000_000_000.0) as u32,
    )
    .unwrap_or_else(chrono::Utc::now);

    TriggerEnvelope {
        source,
        external_id: trigger.id.clone(),
        display_label: trigger.display_label.clone(),
        payload,
        received_at,
        card_link: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious_triggers::types::{DedupeKey, TriggerPayload};

    fn trigger(priority: TriggerPriority) -> Trigger {
        Trigger {
            id: "id".into(),
            source: TriggerSource::ComposioWebhook {
                toolkit: "gmail".into(),
                trigger: "GMAIL_NEW".into(),
                metadata_id: "m".into(),
            },
            display_label: "composio/gmail/GMAIL_NEW".into(),
            payload: TriggerPayload {
                gate_summary: "Composio webhook (2 fields).".into(),
                raw: serde_json::Value::Null,
            },
            priority,
            dedupe_key: DedupeKey("k".into()),
            external_content: true,
            received_at: 0.0,
        }
    }

    fn decision(action: TriageAction, prompt: Option<&str>) -> TriageDecision {
        TriageDecision {
            action,
            target_agent: prompt.map(|_| "orchestrator".to_string()),
            prompt: prompt.map(|p| p.to_string()),
            reason: "because".into(),
        }
    }

    #[test]
    fn drop_maps_to_non_ack_drop() {
        let g = map_triage_to_gate(
            &decision(TriageAction::Drop, None),
            &trigger(TriggerPriority::Normal),
        );
        assert_eq!(
            g,
            GateDecision::Drop {
                acknowledge: false,
                reason: "because".into()
            }
        );
    }

    #[test]
    fn acknowledge_maps_to_ack_drop() {
        let g = map_triage_to_gate(
            &decision(TriageAction::Acknowledge, None),
            &trigger(TriggerPriority::Normal),
        );
        assert_eq!(
            g,
            GateDecision::Drop {
                acknowledge: true,
                reason: "because".into()
            }
        );
    }

    #[test]
    fn react_promotes_keeping_priority() {
        let g = map_triage_to_gate(
            &decision(TriageAction::React, Some("send an ack")),
            &trigger(TriggerPriority::Normal),
        );
        match g {
            GateDecision::Promote {
                priority,
                synthesized_summary,
                ..
            } => {
                assert_eq!(priority, TriggerPriority::Normal);
                assert!(synthesized_summary.contains("send an ack"));
                assert!(synthesized_summary.contains("composio/gmail/GMAIL_NEW"));
            }
            other => panic!("expected promote, got {other:?}"),
        }
    }

    #[test]
    fn escalate_promotes_at_least_high() {
        let g = map_triage_to_gate(
            &decision(TriageAction::Escalate, Some("draft a reply")),
            &trigger(TriggerPriority::Low),
        );
        match g {
            GateDecision::Promote { priority, .. } => {
                assert_eq!(priority, TriggerPriority::High);
            }
            other => panic!("expected promote, got {other:?}"),
        }
    }

    #[test]
    fn escalate_keeps_urgent_when_already_urgent() {
        let g = map_triage_to_gate(
            &decision(TriageAction::Escalate, Some("x")),
            &trigger(TriggerPriority::Urgent),
        );
        match g {
            GateDecision::Promote { priority, .. } => {
                assert_eq!(priority, TriggerPriority::Urgent);
            }
            other => panic!("expected promote, got {other:?}"),
        }
    }

    // ── promotion budget ────────────────────────────────────────────────

    #[test]
    fn budget_allows_up_to_cap_then_downgrades() {
        let budget = PromotionBudget::new(2);
        let promote = || GateDecision::Promote {
            synthesized_summary: "s".into(),
            priority: TriggerPriority::High,
            reason: "r".into(),
        };
        assert!(apply_budget(promote(), &budget, 0.0).is_promote());
        assert!(apply_budget(promote(), &budget, 0.0).is_promote());
        // Third within the hour → downgraded to ack-drop.
        match apply_budget(promote(), &budget, 0.0) {
            GateDecision::Drop {
                acknowledge,
                reason,
            } => {
                assert!(acknowledge);
                assert!(reason.contains("budget exhausted"));
            }
            other => panic!("expected drop, got {other:?}"),
        }
    }

    #[test]
    fn budget_refills_after_an_hour() {
        let budget = PromotionBudget::new(1);
        let promote = || GateDecision::Promote {
            synthesized_summary: "s".into(),
            priority: TriggerPriority::High,
            reason: "r".into(),
        };
        assert!(apply_budget(promote(), &budget, 0.0).is_promote());
        assert!(!apply_budget(promote(), &budget, 100.0).is_promote());
        // One hour later the window slides → promote again.
        assert!(apply_budget(promote(), &budget, 3700.0).is_promote());
    }

    #[test]
    fn budget_zero_blocks_all_promotions() {
        let budget = PromotionBudget::new(0);
        let g = apply_budget(
            GateDecision::Promote {
                synthesized_summary: "s".into(),
                priority: TriggerPriority::High,
                reason: "r".into(),
            },
            &budget,
            0.0,
        );
        assert!(!g.is_promote());
    }

    #[test]
    fn apply_budget_passes_drops_through_untouched() {
        let budget = PromotionBudget::new(5);
        let g = apply_budget(
            GateDecision::Drop {
                acknowledge: false,
                reason: "noise".into(),
            },
            &budget,
            0.0,
        );
        assert_eq!(
            g,
            GateDecision::Drop {
                acknowledge: false,
                reason: "noise".into()
            }
        );
        // A pass-through drop must not consume a promotion slot.
        assert_eq!(budget.used(0.0), 0);
    }
}
