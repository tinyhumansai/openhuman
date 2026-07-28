//! Tests for connector-aware automatic recall.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::openhuman::memory::{MemoryCategory, NamespaceSummary};

/// Memory whose recall is a per-namespace lookup table, so a test states
/// exactly what each namespace holds. Records the namespaces it was asked for.
struct NamespacedMemory {
    hits: HashMap<Option<String>, Vec<MemoryEntry>>,
    summaries: Vec<NamespaceSummary>,
    asked: Mutex<Vec<Option<String>>>,
    fail_summaries: bool,
}

impl NamespacedMemory {
    fn new(hits: Vec<(Option<&str>, Vec<MemoryEntry>)>, counts: Vec<(&str, usize)>) -> Self {
        Self {
            hits: hits
                .into_iter()
                .map(|(namespace, entries)| (namespace.map(str::to_string), entries))
                .collect(),
            summaries: counts
                .into_iter()
                .map(|(namespace, count)| NamespaceSummary {
                    namespace: namespace.to_string(),
                    count,
                    last_updated: None,
                })
                .collect(),
            asked: Mutex::new(Vec::new()),
            fail_summaries: false,
        }
    }

    fn asked_namespaces(&self) -> Vec<Option<String>> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl Memory for NamespacedMemory {
    fn name(&self) -> &str {
        "namespaced-mock"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let namespace = opts.namespace.map(str::to_string);
        self.asked.lock().unwrap().push(namespace.clone());
        Ok(self.hits.get(&namespace).cloned().unwrap_or_default())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        if self.fail_summaries {
            anyhow::bail!("namespace listing unavailable");
        }
        Ok(self.summaries.clone())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn entry(namespace: Option<&str>, key: &str, score: f64) -> MemoryEntry {
    MemoryEntry {
        id: key.into(),
        key: key.into(),
        content: format!("content of {key}"),
        namespace: namespace.map(str::to_string),
        category: MemoryCategory::Conversation,
        timestamp: "now".into(),
        session_id: None,
        score: Some(score),
        taint: Default::default(),
    }
}

#[tokio::test]
async fn a_connector_hit_reaches_the_turn_context() {
    // The bug in one test: the email lives in `skill-gmail`, the turn asks
    // about it, and global-only recall never sees it.
    let mem = NamespacedMemory::new(
        vec![
            (None, vec![entry(None, "chat-note", 0.20)]),
            (
                Some("skill-gmail"),
                vec![entry(Some("skill-gmail"), "gmail:colorado", 0.80)],
            ),
        ],
        vec![("skill-gmail", 12), ("global", 40)],
    );

    let entries = recall_with_connectors(&mem, "colorado", 5).await;
    let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();

    assert!(
        keys.contains(&"gmail:colorado"),
        "connector hit must surface"
    );
    // Ranked on score, not on which namespace it came from.
    assert_eq!(keys.first(), Some(&"gmail:colorado"));
    assert!(keys.contains(&"chat-note"), "global hits are not displaced");

    // Only connector namespaces are fanned out to; `global` is the default
    // recall, not a second explicit one.
    let asked = mem.asked_namespaces();
    assert_eq!(asked.iter().filter(|ns| ns.is_none()).count(), 1);
    assert!(asked.contains(&Some("skill-gmail".to_string())));
    assert!(
        !asked.contains(&Some("global".to_string())),
        "global must not be searched twice: {asked:?}"
    );
}

#[tokio::test]
async fn the_fan_out_is_bounded_to_the_busiest_namespaces() {
    let counts = vec![
        ("skill-a", 1),
        ("skill-b", 900),
        ("skill-c", 50),
        ("skill-d", 700),
        ("skill-e", 300),
        ("skill-empty", 0),
    ];
    let mem = NamespacedMemory::new(vec![(None, Vec::new())], counts);

    let _ = recall_with_connectors(&mem, "anything", 5).await;
    let asked: Vec<String> = mem
        .asked_namespaces()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(
        asked.len(),
        MAX_CONNECTOR_NAMESPACES,
        "an unbounded fan-out would scan the whole store every turn: {asked:?}"
    );
    // Busiest first, and an empty namespace is never worth a scan.
    assert_eq!(asked, vec!["skill-b", "skill-d", "skill-e", "skill-c"]);
    assert!(!asked.contains(&"skill-empty".to_string()));
}

#[tokio::test]
async fn results_are_deduplicated_and_capped_at_the_limit() {
    let mem = NamespacedMemory::new(
        vec![
            (
                None,
                vec![entry(None, "dupe", 0.5), entry(None, "global-2", 0.4)],
            ),
            (
                Some("skill-gmail"),
                vec![
                    // Same (namespace, key) twice — one slot, not two.
                    entry(None, "dupe", 0.5),
                    entry(Some("skill-gmail"), "gmail-1", 0.9),
                ],
            ),
        ],
        vec![("skill-gmail", 5)],
    );

    let entries = recall_with_connectors(&mem, "q", 2).await;
    assert_eq!(entries.len(), 2, "the caller's limit is respected");
    let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, vec!["gmail-1", "dupe"]);
}

#[tokio::test]
async fn a_store_that_cannot_list_namespaces_degrades_to_global_only() {
    let mut mem = NamespacedMemory::new(
        vec![(None, vec![entry(None, "chat-note", 0.3)])],
        vec![("skill-gmail", 12)],
    );
    mem.fail_summaries = true;

    let entries = recall_with_connectors(&mem, "q", 5).await;
    assert_eq!(entries.len(), 1, "global recall still returns its hits");
    assert_eq!(
        mem.asked_namespaces(),
        vec![None],
        "no connector recall is attempted when the listing fails"
    );
}
