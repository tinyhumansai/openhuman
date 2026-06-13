//! Retrieval: the seed. Hydrate a `(repo, ref)` working set from the store,
//! score it with **BM25 (lexical) ∪ dense (cosine)**, **RRF-fuse**, and report
//! a **coverage** flag (`full`/`partial`/`none`) so callers know whether the
//! index is complete or the agent should lean on grep.
//!
//! BM25 is in-memory over the hydrated tokens (the working set is one repo's
//! files — small; this matches the validated prototype and keeps the
//! hydrate-per-query model simple). Dense is cosine over the L2-normalised
//! structural-aug vectors. The query is embedded once with the same provider
//! the index was built with (its `signature()` is the cache `model` key).

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::openhuman::embeddings::EmbeddingProvider;

use super::index::code_tokens;
use super::store::{BlobEntry, CodegraphStore};

const RRF_K: f32 = 60.0;
const PER_ARM: usize = 20; // top-N from each arm fed into RRF
const BM25_K1: f32 = 1.5;
const BM25_B: f32 = 0.75;

/// How complete the index is for the queried `(repo, ref)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// Every manifest file is embedded — trust the candidates.
    Full,
    /// Some files still pending (background index in flight) — treat as hints.
    Partial,
    /// Nothing indexed yet — fall back to grep.
    None,
}

/// The seed result: ranked candidate paths + how complete the index was.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchOutcome {
    pub hits: Vec<String>,
    pub coverage: Coverage,
    /// Files embedded (hydrated) vs total in the manifest.
    pub indexed: usize,
    pub total: usize,
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// BM25-Okapi over the hydrated docs; returns doc indices ranked best-first.
fn bm25_rank(docs: &[BlobEntry], query: &[String]) -> Vec<usize> {
    let n = docs.len() as f32;
    let lens: Vec<f32> = docs.iter().map(|d| d.tokens.len() as f32).collect();
    let avgdl = (lens.iter().sum::<f32>() / n).max(1.0);
    // per-doc term frequency tables
    let tfs: Vec<HashMap<&str, f32>> = docs
        .iter()
        .map(|d| {
            let mut m: HashMap<&str, f32> = HashMap::new();
            for w in &d.tokens {
                *m.entry(w.as_str()).or_insert(0.0) += 1.0;
            }
            m
        })
        .collect();
    let q_terms: HashSet<&str> = query.iter().map(|s| s.as_str()).collect();

    let mut scores = vec![0.0f32; docs.len()];
    for &t in &q_terms {
        let df = tfs.iter().filter(|m| m.contains_key(t)).count() as f32;
        if df == 0.0 {
            continue;
        }
        let idf = (((n - df + 0.5) / (df + 0.5)) + 1.0).ln();
        for (i, m) in tfs.iter().enumerate() {
            if let Some(&f) = m.get(t) {
                let denom = f + BM25_K1 * (1.0 - BM25_B + BM25_B * lens[i] / avgdl);
                scores[i] += idf * (f * (BM25_K1 + 1.0)) / denom;
            }
        }
    }
    rank_by_score(&scores)
}

/// Cosine (dot over normalised vectors) of `qv` against each doc; best-first.
fn dense_rank(docs: &[BlobEntry], qv: &[f32]) -> Vec<usize> {
    let scores: Vec<f32> = docs
        .iter()
        .map(|d| d.emb.iter().zip(qv).map(|(a, b)| a * b).sum::<f32>())
        .collect();
    rank_by_score(&scores)
}

fn rank_by_score(scores: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    idx
}

/// Reciprocal-rank fusion of several rankings (top-`PER_ARM` of each), top-`k`.
fn rrf(rankings: &[Vec<usize>], k: usize) -> Vec<usize> {
    let mut score: HashMap<usize, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, &doc) in ranking.iter().take(PER_ARM).enumerate() {
            *score.entry(doc).or_insert(0.0) += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut items: Vec<(usize, f32)> = score.into_iter().collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    items.into_iter().take(k).map(|(i, _)| i).collect()
}

/// Seed `query` against a `(repo, ref)` index: BM25 ∪ dense, RRF-fused, top-`k`,
/// with a coverage flag. Embeds the query once with `embedder`.
pub async fn search_ref(
    store: &mut CodegraphStore,
    repo_id: &str,
    git_ref: &str,
    query: &str,
    embedder: &dyn EmbeddingProvider,
    k: usize,
) -> Result<SearchOutcome> {
    let total = store.manifest_size(repo_id, git_ref)?;
    // Auto-detect the index mode: prefer the dense arm (rows under the
    // embedder's signature); if none, fall back to the lexical-only key (a
    // small repo indexed BM25-only). Lexical search makes no embedder call.
    let dense_model = embedder.signature();
    let mut docs = store.hydrate(repo_id, git_ref, &dense_model)?;
    let dense_active = !docs.is_empty();
    if !dense_active {
        docs = store.hydrate(repo_id, git_ref, super::index::LEXICAL_MODEL)?;
    }

    let coverage = if total == 0 {
        Coverage::None
    } else if docs.len() >= total {
        Coverage::Full
    } else {
        Coverage::Partial
    };
    if docs.is_empty() {
        return Ok(SearchOutcome {
            hits: vec![],
            coverage,
            indexed: 0,
            total,
        });
    }

    let q_tokens = code_tokens(query);
    let bm = bm25_rank(&docs, &q_tokens);

    // Dense arm only when the index has vectors — otherwise BM25 alone, and no
    // query-embed round-trip. RRF over a single ranking preserves its order.
    let arms = if dense_active {
        let mut qv = embedder
            .embed(&[query])
            .await
            .context("codegraph: embed query")?
            .into_iter()
            .next()
            .unwrap_or_default();
        l2_normalize(&mut qv);
        vec![bm, dense_rank(&docs, &qv)]
    } else {
        vec![bm]
    };

    let fused = rrf(&arms, k);
    let hits = fused.into_iter().map(|i| docs[i].path.clone()).collect();
    Ok(SearchOutcome {
        hits,
        coverage,
        indexed: docs.len(),
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tempfile::TempDir;

    fn doc(path: &str, toks: &[&str]) -> BlobEntry {
        BlobEntry {
            path: path.into(),
            tokens: toks.iter().map(|s| s.to_string()).collect(),
            emb: vec![0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn bm25_ranks_the_matching_doc_first() {
        let docs = vec![
            doc("auth.rs", &["login", "session", "token"]),
            doc("retry.rs", &["reconcile", "backoff", "charge"]),
            doc("util.rs", &["helper", "misc"]),
        ];
        let ranked = bm25_rank(&docs, &code_tokens("reconcile after backoff"));
        assert_eq!(ranked[0], 1, "retry.rs ranks first for 'reconcile/backoff'");
    }

    #[test]
    fn rrf_blends_two_rankings() {
        // bm25 likes doc 2, dense likes doc 0; both should surface above doc 1.
        let fused = rrf(&[vec![2, 1, 0], vec![0, 1, 2]], 3);
        assert!(fused.contains(&0) && fused.contains(&2));
        assert_eq!(fused.len(), 3);
    }

    struct FakeEmbedder;
    #[async_trait]
    impl EmbeddingProvider for FakeEmbedder {
        fn name(&self) -> &str {
            "fake"
        }
        fn model_id(&self) -> &str {
            "fake-1"
        }
        fn dimensions(&self) -> usize {
            3
        }
        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    #[tokio::test]
    async fn search_ref_returns_ranked_hits_and_partial_coverage() {
        let tmp = TempDir::new().unwrap();
        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();
        let sig = FakeEmbedder.signature();
        store
            .put_blob(
                "a",
                &sig,
                &["reconcile".into(), "backoff".into()],
                &[1.0, 0.0, 0.0],
            )
            .unwrap();
        store
            .put_blob(
                "b",
                &sig,
                &["login".into(), "token".into()],
                &[0.0, 1.0, 0.0],
            )
            .unwrap();
        // manifest has a 3rd file with no cached blob → partial coverage.
        store
            .set_manifest(
                "r",
                "main",
                &[
                    ("retry.rs".into(), "a".into()),
                    ("auth.rs".into(), "b".into()),
                    ("pending.rs".into(), "uncached".into()),
                ],
            )
            .unwrap();

        let out = search_ref(
            &mut store,
            "r",
            "main",
            "reconcile backoff",
            &FakeEmbedder,
            10,
        )
        .await
        .unwrap();
        assert_eq!(out.coverage, Coverage::Partial);
        assert_eq!(out.indexed, 2);
        assert_eq!(out.total, 3);
        assert_eq!(out.hits[0], "retry.rs", "lexical match surfaces first");
    }
}
