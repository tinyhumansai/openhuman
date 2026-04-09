//! Decision log for tracking what the subconscious has already surfaced.
//! Prevents re-escalating the same state changes across ticks.

use super::types::TickDecision;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// TTL for decision records before auto-expiry (24 hours).
const RECORD_TTL_SECS: f64 = 24.0 * 60.0 * 60.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub tick_at: f64,
    pub decision: TickDecision,
    pub source_doc_ids: Vec<String>,
    pub reason: String,
    pub acknowledged: bool,
    pub expires_at: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionLog {
    records: Vec<DecisionRecord>,
}

impl DecisionLog {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn was_already_surfaced(&self, doc_ids: &[String]) -> bool {
        let now = now_secs();
        self.records.iter().any(|r| {
            !r.acknowledged
                && r.expires_at > now
                && r.decision != TickDecision::Noop
                && r.source_doc_ids.iter().any(|id| doc_ids.contains(id))
        })
    }

    pub fn filter_unsurfaced(&self, doc_ids: &[String]) -> Vec<String> {
        let surfaced: HashSet<&str> = self
            .records
            .iter()
            .filter(|r| {
                !r.acknowledged && r.expires_at > now_secs() && r.decision != TickDecision::Noop
            })
            .flat_map(|r| r.source_doc_ids.iter().map(|s| s.as_str()))
            .collect();

        doc_ids
            .iter()
            .filter(|id| !surfaced.contains(id.as_str()))
            .cloned()
            .collect()
    }

    pub fn record(
        &mut self,
        tick_at: f64,
        decision: TickDecision,
        reason: &str,
        source_doc_ids: Vec<String>,
    ) {
        self.records.push(DecisionRecord {
            tick_at,
            decision,
            source_doc_ids,
            reason: reason.to_string(),
            acknowledged: false,
            expires_at: tick_at + RECORD_TTL_SECS,
        });
    }

    pub fn mark_acknowledged(&mut self, doc_ids: &[String]) {
        for record in &mut self.records {
            if record.source_doc_ids.iter().any(|id| doc_ids.contains(id)) {
                record.acknowledged = true;
            }
        }
    }

    pub fn prune_expired(&mut self) {
        let now = now_secs();
        self.records.retain(|r| r.expires_at > now);
    }

    pub fn active_count(&self) -> usize {
        let now = now_secs();
        self.records.iter().filter(|r| r.expires_at > now).count()
    }

    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("serialize decision log: {e}"))
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("deserialize decision log: {e}"))
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> f64 {
        now_secs()
    }

    #[test]
    fn empty_log_surfaces_nothing() {
        let log = DecisionLog::new();
        assert!(!log.was_already_surfaced(&["doc-1".into()]));
    }

    #[test]
    fn recorded_escalation_is_surfaced() {
        let mut log = DecisionLog::new();
        log.record(
            now(),
            TickDecision::Escalate,
            "deadline",
            vec!["doc-1".into()],
        );
        assert!(log.was_already_surfaced(&["doc-1".into()]));
        assert!(!log.was_already_surfaced(&["doc-2".into()]));
    }

    #[test]
    fn noop_decisions_are_not_surfaced() {
        let mut log = DecisionLog::new();
        log.record(now(), TickDecision::Noop, "nothing", vec!["doc-1".into()]);
        assert!(!log.was_already_surfaced(&["doc-1".into()]));
    }

    #[test]
    fn acknowledged_records_are_not_surfaced() {
        let mut log = DecisionLog::new();
        log.record(
            now(),
            TickDecision::Escalate,
            "deadline",
            vec!["doc-1".into()],
        );
        log.mark_acknowledged(&["doc-1".into()]);
        assert!(!log.was_already_surfaced(&["doc-1".into()]));
    }

    #[test]
    fn expired_records_are_not_surfaced() {
        let mut log = DecisionLog::new();
        let old_time = now() - RECORD_TTL_SECS - 1.0;
        log.record(
            old_time,
            TickDecision::Escalate,
            "old",
            vec!["doc-1".into()],
        );
        assert!(!log.was_already_surfaced(&["doc-1".into()]));
    }

    #[test]
    fn prune_removes_expired() {
        let mut log = DecisionLog::new();
        let old_time = now() - RECORD_TTL_SECS - 1.0;
        log.record(
            old_time,
            TickDecision::Escalate,
            "old",
            vec!["doc-1".into()],
        );
        log.record(now(), TickDecision::Act, "new", vec!["doc-2".into()]);
        assert_eq!(log.records().len(), 2);
        log.prune_expired();
        assert_eq!(log.records().len(), 1);
        assert_eq!(log.records()[0].source_doc_ids, vec!["doc-2".to_string()]);
    }

    #[test]
    fn filter_unsurfaced_returns_new_docs() {
        let mut log = DecisionLog::new();
        log.record(now(), TickDecision::Escalate, "seen", vec!["doc-1".into()]);
        let unsurfaced = log.filter_unsurfaced(&["doc-1".into(), "doc-2".into(), "doc-3".into()]);
        assert_eq!(unsurfaced, vec!["doc-2".to_string(), "doc-3".to_string()]);
    }

    #[test]
    fn roundtrip_json() {
        let mut log = DecisionLog::new();
        log.record(now(), TickDecision::Escalate, "test", vec!["doc-1".into()]);
        let json = log.to_json().unwrap();
        let restored = DecisionLog::from_json(&json).unwrap();
        assert_eq!(restored.records().len(), 1);
        assert_eq!(restored.records()[0].reason, "test");
    }
}
