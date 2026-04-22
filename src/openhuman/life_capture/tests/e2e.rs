//! End-to-end test for the life-capture pipeline:
//!   raw text → quote-strip → redact → upsert → embed → vector store →
//!   hybrid search returns the redacted/quote-stripped item.
//!
//! Uses a deterministic FakeEmbedder so the test stays hermetic — no network.

use crate::openhuman::life_capture::embedder::Embedder;
use crate::openhuman::life_capture::index::{IndexReader, IndexWriter, PersonalIndex};
use crate::openhuman::life_capture::types::{Item, Query, Source};
use crate::openhuman::life_capture::{quote_strip, redact};
use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

/// Hashes each input string into a sparse 1536-dim vector — same input always
/// produces the same vector, so the cosine-style match in vec0 finds it back.
struct FakeEmbedder;

#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0_f32; 1536];
                for (i, b) in t.as_bytes().iter().enumerate().take(64) {
                    v[(*b as usize) % 1536] += 1.0 / (1.0 + i as f32);
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> usize {
        1536
    }
}

#[tokio::test]
async fn ingest_then_retrieve_with_redaction_and_quote_strip() {
    let idx = PersonalIndex::open_in_memory().await.unwrap();
    let writer = IndexWriter::new(&idx);
    let embedder = FakeEmbedder;

    let raw = "The Ledger contract draft is ready, sarah@example.com signed.\n\n\
               On Mon, Apr 21, 2026 at 9:14 AM, Sarah <sarah@x> wrote:\n\
               > earlier text we don't want indexed";
    let cleaned = redact::redact(&quote_strip::strip_quoted_reply(raw));
    assert!(!cleaned.contains("earlier text"), "quote-strip didn't drop the quoted block");
    assert!(cleaned.contains("<EMAIL>"), "redact didn't mask the email");

    let item = Item {
        id: Uuid::new_v4(),
        source: Source::Gmail,
        external_id: "msg-1".into(),
        ts: Utc::now(),
        author: None,
        subject: Some("Ledger contract".into()),
        text: cleaned,
        metadata: serde_json::json!({}),
    };
    writer.upsert(&[item.clone()]).await.unwrap();

    let vecs = embedder.embed_batch(&[item.text.as_str()]).await.unwrap();
    writer.upsert_vector(&item.id, &vecs[0]).await.unwrap();

    let reader = IndexReader::new(&idx);
    let q = Query::simple("ledger contract", 5);
    let qvec = embedder
        .embed_batch(&[q.text.as_str()])
        .await
        .unwrap()
        .remove(0);
    let hits = reader.hybrid_search(&q, &qvec).await.unwrap();
    assert_eq!(hits.len(), 1, "expected exactly one hit");
    assert_eq!(hits[0].item.external_id, "msg-1");
    assert!(
        !hits[0].item.text.contains("earlier text"),
        "quoted reply leaked through to indexed text"
    );
    assert!(
        hits[0].item.text.contains("<EMAIL>"),
        "redacted token missing from stored text"
    );
}
