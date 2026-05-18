//! Integration-style unit tests for the transcript ingestion pipeline.
//!
//! Uses an in-memory [`Memory`] mock so the pipeline can be exercised
//! end-to-end without a SQLite/vector backend.

use super::*;
use crate::openhuman::agent::harness::session::transcript::{SessionTranscript, TranscriptMeta};
use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;

/// Tiny in-memory `Memory` implementation good enough to drive the
/// transcript-ingest pipeline. Not exposed outside tests.
struct InMemory {
    entries: Mutex<Vec<MemoryEntry>>,
}

impl InMemory {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<MemoryEntry> {
        self.entries.lock().unwrap().clone()
    }
}

#[async_trait]
impl Memory for InMemory {
    fn name(&self) -> &str {
        "in_memory_test"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut e = self.entries.lock().unwrap();
        // Replace-on-collision so re-ingest is idempotent.
        if let Some(existing) = e
            .iter_mut()
            .find(|e| e.namespace.as_deref() == Some(namespace) && e.key == key)
        {
            existing.content = content.to_string();
            existing.timestamp = "2026-05-09T12:00:00Z".to_string();
            return Ok(());
        }
        e.push(MemoryEntry {
            id: format!("id-{}-{}", namespace, key),
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-05-09T12:00:00Z".to_string(),
            session_id: session_id.map(|s| s.to_string()),
            score: None,
        });
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let q = query.to_ascii_lowercase();
        let entries = self.entries.lock().unwrap().clone();
        let mut hits: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| {
                opts.namespace
                    .map(|n| e.namespace.as_deref() == Some(n))
                    .unwrap_or(true)
            })
            .filter(|e| e.content.to_ascii_lowercase().contains(&q) || q.is_empty())
            .map(|mut e| {
                e.score = Some(1.0);
                e
            })
            .collect();
        hits.truncate(limit);
        Ok(hits)
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.namespace.as_deref() == Some(namespace) && e.key == key)
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                namespace
                    .map(|n| e.namespace.as_deref() == Some(n))
                    .unwrap_or(true)
            })
            .cloned()
            .collect())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.lock().unwrap().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn fake_meta(thread_id: Option<&str>) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "main".into(),
        dispatcher: "native".into(),
        created: "2026-05-09T11:00:00Z".into(),
        updated: "2026-05-09T12:00:00Z".into(),
        turn_count: 4,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: thread_id.map(|s| s.into()),
    }
}

#[tokio::test]
async fn ingest_extracts_high_importance_preference_with_provenance() {
    let mem = InMemory::new();
    let transcript = SessionTranscript {
        meta: fake_meta(Some("thr_alpha")),
        messages: vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
            ChatMessage::user("I prefer Postgres over MySQL for any new metadata service we ship."),
            ChatMessage::user("Still need to migrate the auth service before Friday."),
        ],
    };

    let report =
        ingest_session_transcript(&mem, &transcript, &PathBuf::from("/tmp/123_main.jsonl"))
            .await
            .expect("ingest must succeed");

    assert!(report.extracted >= 2, "report: {:?}", report);
    assert!(report.stored >= 2);

    let stored = mem.snapshot();
    assert!(stored.iter().any(
        |e| e.namespace.as_deref() == Some(CONVERSATION_MEMORY_NAMESPACE)
            && e.key.starts_with("high.preference.")
            && e.content.contains("Postgres")
            && e.content.contains("[provenance]")
            && e.content.contains("thr_alpha")
    ));
    assert!(stored
        .iter()
        .any(|e| e.key.starts_with("med.unresolved_task.") && e.content.contains("Friday")));
}

#[tokio::test]
async fn re_ingest_is_idempotent() {
    let mem = InMemory::new();
    let transcript = SessionTranscript {
        meta: fake_meta(Some("thr_beta")),
        messages: vec![ChatMessage::user(
            "I prefer Postgres for everything new — please default to it.",
        )],
    };
    let path = PathBuf::from("/tmp/200_main.jsonl");

    let r1 = ingest_session_transcript(&mem, &transcript, &path)
        .await
        .unwrap();
    let r2 = ingest_session_transcript(&mem, &transcript, &path)
        .await
        .unwrap();

    assert_eq!(r1.stored, 1);
    assert_eq!(r2.stored, 0, "second pass must dedupe everything");
    assert!(r2.deduped >= 1);
    assert_eq!(mem.snapshot().len(), 1);
}

#[tokio::test]
async fn ingest_captures_user_reflection_and_recurring_pattern() {
    let mem = InMemory::new();
    let transcript = SessionTranscript {
        meta: fake_meta(Some("thr_gamma")),
        messages: vec![
            ChatMessage::user("I prefer terse responses with no preamble."),
            ChatMessage::user("Going forward I want code-first answers."),
            ChatMessage::user("I always want bullet points when listing options."),
            ChatMessage::user(
                "I realized we keep reintroducing the same schema bug — \
                 next time write a regression test first.",
            ),
        ],
    };

    let report =
        ingest_session_transcript(&mem, &transcript, &PathBuf::from("/tmp/300_main.jsonl"))
            .await
            .unwrap();

    assert!(
        report.reflections_extracted >= 2,
        "expected at least one explicit + one recurring reflection: {:?}",
        report
    );
    assert!(report.reflections_stored >= 2);
    let stored = mem.snapshot();
    assert!(stored.iter().any(|e| e.namespace.as_deref()
        == Some(CONVERSATION_REFLECTIONS_NAMESPACE)
        && e.key.contains("user_reflection")));
    assert!(stored.iter().any(|e| e.namespace.as_deref()
        == Some(CONVERSATION_REFLECTIONS_NAMESPACE)
        && e.key.contains("recurring_preferences")));
}

#[tokio::test]
async fn ingest_filters_low_signal_chatter() {
    let mem = InMemory::new();
    let transcript = SessionTranscript {
        meta: fake_meta(None),
        messages: vec![
            ChatMessage::user("ok"),
            ChatMessage::user("thanks!"),
            ChatMessage::assistant("👍"),
            ChatMessage::user("hi there"),
        ],
    };

    let report =
        ingest_session_transcript(&mem, &transcript, &PathBuf::from("/tmp/400_main.jsonl"))
            .await
            .unwrap();

    assert_eq!(report.extracted, 0);
    assert_eq!(report.stored, 0);
    assert!(mem.snapshot().is_empty());
}
