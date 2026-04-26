//! Retrieval eval for life_capture.
//!
//! Loads `tests/fixtures/life_capture/corpus.json` into an in-memory
//! `PersonalIndex`, computes deterministic toy embeddings (feature-hashing,
//! 1536 dims) for every item and query, then runs each query through the
//! keyword / vector / hybrid path according to its `kind` and asserts
//! `must_contain` / `must_not_contain` within the requested top-K prefix.
//!
//! The fixture shape reserves an optional `relevant` field per query so a
//! future recall@k / MRR harness can consume the same JSON without a rewrite.
//!
//! Run with: `cargo test --test life_capture_retrieval_eval`

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use openhuman_core::openhuman::life_capture::index::{IndexReader, IndexWriter, PersonalIndex};
use openhuman_core::openhuman::life_capture::types::{Item, Query, Source};

const EMBED_DIM: usize = 1536;
const FIXTURE_JSON: &str = include_str!("fixtures/life_capture/corpus.json");

// ─── Fixture shape ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Corpus {
    #[allow(dead_code)]
    version: u32,
    items: Vec<FixtureItem>,
    queries: Vec<FixtureQuery>,
}

#[derive(Debug, Deserialize)]
struct FixtureItem {
    ext_id: String,
    source: Source,
    ts: DateTime<Utc>,
    subject: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
struct FixtureQuery {
    id: String,
    text: String,
    kind: QueryKind,
    k: usize,
    #[serde(default)]
    sources: Vec<Source>,
    #[serde(default)]
    since: Option<DateTime<Utc>>,
    #[serde(default)]
    until: Option<DateTime<Utc>>,
    #[serde(default)]
    must_contain_in_top: Option<usize>,
    #[serde(default)]
    must_contain: Vec<String>,
    #[serde(default)]
    must_not_contain_in_top: Option<usize>,
    #[serde(default)]
    must_not_contain: Vec<String>,
    #[serde(default)]
    pending: bool,
    #[allow(dead_code)]
    #[serde(default)]
    pending_reason: Option<String>,
    /// Graded relevance labels used by the recall@k / MRR aggregator.
    /// Empty `relevant` => the query is excluded from those metrics
    /// (e.g. pure negative-assertion queries like q-neg-01).
    #[serde(default)]
    relevant: Vec<RelevantDoc>,
}

#[derive(Debug, Deserialize)]
struct RelevantDoc {
    ext_id: String,
    #[serde(default = "default_grade")]
    #[allow(dead_code)]
    grade: u8,
}

fn default_grade() -> u8 {
    1
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum QueryKind {
    Keyword,
    Semantic,
    Mixed,
}

// ─── Toy embedder: deterministic feature hashing over word tokens ───────
//
// Tokens: lowercase alphanumeric, length ≥ 3. Each token hashes (FNV-1a) to
// a bucket in [0, EMBED_DIM) and increments that bucket. The final vector
// is L2-normalized. Cosine similarity between two embeddings reduces to
// fractional token overlap — deterministic, offline, zero model deps.

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; EMBED_DIM];
    for tok in tokenize(text) {
        let bucket = (fnv1a_64(tok.as_bytes()) as usize) % EMBED_DIM;
        v[bucket] += 1.0;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

fn item_text_for_embedding(it: &FixtureItem) -> String {
    match &it.subject {
        Some(s) => format!("{} {}", s, it.text),
        None => it.text.clone(),
    }
}

// ─── Runner ─────────────────────────────────────────────────────────────

fn fixture_to_item(fx: &FixtureItem) -> Item {
    Item {
        id: Uuid::new_v4(),
        source: fx.source,
        external_id: fx.ext_id.clone(),
        ts: fx.ts,
        author: None,
        subject: fx.subject.clone(),
        text: fx.text.clone(),
        metadata: serde_json::json!({}),
    }
}

/// Find the ext_id of each hit by mapping its UUID back via `item.external_id`.
fn hit_ext_ids(hits: &[openhuman_core::openhuman::life_capture::types::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.item.external_id.clone()).collect()
}

fn assert_contains_in_top(
    query_id: &str,
    hits_ext: &[String],
    top_n: usize,
    expected: &[String],
) -> Result<()> {
    let prefix: Vec<&String> = hits_ext.iter().take(top_n).collect();
    for want in expected {
        if !prefix.iter().any(|got| *got == want) {
            return Err(anyhow!(
                "[{query_id}] expected `{want}` within top-{top_n}, got: {:?}",
                prefix
            ));
        }
    }
    Ok(())
}

fn assert_absent_in_top(
    query_id: &str,
    hits_ext: &[String],
    top_n: usize,
    forbidden: &[String],
) -> Result<()> {
    let prefix: Vec<&String> = hits_ext.iter().take(top_n).collect();
    for bad in forbidden {
        if prefix.iter().any(|got| *got == bad) {
            return Err(anyhow!(
                "[{query_id}] forbidden `{bad}` appeared within top-{top_n}: {:?}",
                prefix
            ));
        }
    }
    Ok(())
}

async fn run_query(reader: &IndexReader, q: &FixtureQuery) -> Result<Vec<String>> {
    let hits = match q.kind {
        QueryKind::Keyword => reader
            .keyword_search(&q.text, q.k)
            .await
            .with_context(|| format!("keyword_search failed for {}", q.id))?,
        QueryKind::Semantic => {
            let v = embed(&q.text);
            reader
                .vector_search(&v, q.k)
                .await
                .with_context(|| format!("vector_search failed for {}", q.id))?
        }
        QueryKind::Mixed => {
            let v = embed(&q.text);
            let query = Query {
                text: q.text.clone(),
                k: q.k,
                sources: q.sources.clone(),
                since: q.since,
                until: q.until,
            };
            reader
                .hybrid_search(&query, &v)
                .await
                .with_context(|| format!("hybrid_search failed for {}", q.id))?
        }
    };
    Ok(hit_ext_ids(&hits))
}

#[tokio::test]
async fn retrieval_eval_against_fixture_corpus() -> Result<()> {
    let corpus: Corpus = serde_json::from_str(FIXTURE_JSON)
        .context("parsing tests/fixtures/life_capture/corpus.json")?;

    let idx = PersonalIndex::open_in_memory().await?;
    let writer = IndexWriter::new(&idx);

    // Upsert items and vectors.
    let mut items: Vec<Item> = corpus.items.iter().map(fixture_to_item).collect();
    writer.upsert(&mut items).await?;
    for (fx, it) in corpus.items.iter().zip(items.iter()) {
        let v = embed(&item_text_for_embedding(fx));
        writer.upsert_vector(&it.id, &v).await?;
    }

    let reader = IndexReader::new(&idx);

    // Run every query, collect failures, report them together.
    let mut failures: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut metrics: Vec<QueryMetrics> = Vec::new();
    for q in &corpus.queries {
        if q.pending {
            pending.push(q.id.clone());
            continue;
        }
        let hits_ext = match run_query(&reader, q).await {
            Ok(h) => h,
            Err(e) => {
                failures.push(format!("[{}] query errored: {e:#}", q.id));
                continue;
            }
        };

        if !q.must_contain.is_empty() {
            let n = q.must_contain_in_top.unwrap_or(q.k);
            if let Err(e) = assert_contains_in_top(&q.id, &hits_ext, n, &q.must_contain) {
                failures.push(e.to_string());
            }
        }
        if !q.must_not_contain.is_empty() {
            let n = q.must_not_contain_in_top.unwrap_or(q.k);
            if let Err(e) = assert_absent_in_top(&q.id, &hits_ext, n, &q.must_not_contain) {
                failures.push(e.to_string());
            }
        }

        if !q.relevant.is_empty() {
            metrics.push(score_query(q, &hits_ext));
        }
    }

    if !pending.is_empty() {
        eprintln!(
            "[retrieval_eval] {} pending quer{} skipped: {}",
            pending.len(),
            if pending.len() == 1 { "y" } else { "ies" },
            pending.join(", ")
        );
    }
    print_metrics_summary(&metrics);
    if !failures.is_empty() {
        return Err(anyhow!(
            "{} of {} queries failed:\n  - {}",
            failures.len(),
            corpus.queries.len(),
            failures.join("\n  - ")
        ));
    }
    Ok(())
}

// ─── Recall@k + MRR aggregator ──────────────────────────────────────────
//
// `score_query` computes per-query recall@1/3/5 and MRR against the graded
// `relevant` labels in the fixture. Grades are not used today (binary
// relevance), but the field is kept so we can graduate to nDCG without a
// fixture rewrite.

#[derive(Debug, Clone)]
struct QueryMetrics {
    id: String,
    kind: QueryKind,
    recall_at_1: f64,
    recall_at_3: f64,
    recall_at_5: f64,
    mrr: f64,
}

fn score_query(q: &FixtureQuery, hits_ext: &[String]) -> QueryMetrics {
    let relevant: std::collections::HashSet<&str> =
        q.relevant.iter().map(|r| r.ext_id.as_str()).collect();
    let total = relevant.len() as f64;
    let found_at = |k: usize| -> f64 {
        let n = k.min(hits_ext.len());
        hits_ext[..n]
            .iter()
            .filter(|h| relevant.contains(h.as_str()))
            .count() as f64
            / total
    };
    let mrr = hits_ext
        .iter()
        .position(|h| relevant.contains(h.as_str()))
        .map(|pos| 1.0 / (pos as f64 + 1.0))
        .unwrap_or(0.0);
    QueryMetrics {
        id: q.id.clone(),
        kind: q.kind,
        recall_at_1: found_at(1),
        recall_at_3: found_at(3),
        recall_at_5: found_at(5),
        mrr,
    }
}

fn print_metrics_summary(metrics: &[QueryMetrics]) {
    if metrics.is_empty() {
        eprintln!("[retrieval_eval] no queries had `relevant` labels — skipping recall/MRR");
        return;
    }
    eprintln!("\n[retrieval_eval] per-query metrics ({}q):", metrics.len());
    eprintln!(
        "  {:<12} {:<8} {:>6} {:>6} {:>6} {:>6}",
        "id", "kind", "R@1", "R@3", "R@5", "MRR"
    );
    for m in metrics {
        eprintln!(
            "  {:<12} {:<8} {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
            m.id,
            format!("{:?}", m.kind).to_lowercase(),
            m.recall_at_1,
            m.recall_at_3,
            m.recall_at_5,
            m.mrr,
        );
    }
    let n = metrics.len() as f64;
    let avg = |get: fn(&QueryMetrics) -> f64| metrics.iter().map(get).sum::<f64>() / n;
    eprintln!(
        "  {:<12} {:<8} {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
        "MEAN",
        "",
        avg(|m| m.recall_at_1),
        avg(|m| m.recall_at_3),
        avg(|m| m.recall_at_5),
        avg(|m| m.mrr),
    );
    // Per-kind breakdown
    for kind in [QueryKind::Keyword, QueryKind::Semantic, QueryKind::Mixed] {
        let subset: Vec<&QueryMetrics> = metrics.iter().filter(|m| m.kind == kind).collect();
        if subset.is_empty() {
            continue;
        }
        let kn = subset.len() as f64;
        let kavg = |get: fn(&QueryMetrics) -> f64| subset.iter().map(|m| get(*m)).sum::<f64>() / kn;
        eprintln!(
            "  {:<12} {:<8} {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
            format!("MEAN-{:?}", kind).to_lowercase(),
            "",
            kavg(|m| m.recall_at_1),
            kavg(|m| m.recall_at_3),
            kavg(|m| m.recall_at_5),
            kavg(|m| m.mrr),
        );
    }
}

// ─── Sanity tests for the toy embedder ──────────────────────────────────

#[test]
fn embed_is_unit_length_when_tokens_present() {
    let v = embed("Tokyo itinerary draft");
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-5,
        "expected unit vector, got {norm}"
    );
}

#[test]
fn embed_shared_tokens_produce_nonzero_cosine() {
    let a = embed("tokyo hotel park hyatt");
    let b = embed("tokyo trip itinerary");
    let cos: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!(
        cos > 0.0,
        "shared token `tokyo` should yield positive cosine, got {cos}"
    );
}

#[test]
fn embed_disjoint_vocab_gives_zero_cosine() {
    let a = embed("apple banana cherry");
    let b = embed("xenon yacht zebra");
    let cos: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!(
        cos.abs() < 1e-6,
        "disjoint vocab should give zero cosine, got {cos}"
    );
}
