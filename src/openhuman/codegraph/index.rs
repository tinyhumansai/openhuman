//! Indexing: enumerate a git tree's blobs → for each unseen `(content, model)`
//! extract a structural-aug doc + BM25 tokens, embed it, and cache by blob SHA;
//! then write the `(repo, ref)` manifest. Content-addressed + incremental: a
//! branch switch / new commit / pull only (re)embeds the blobs that changed.
//!
//! The structural extractor here is a dependency-free heuristic (signatures +
//! imports + call identifiers + leading doc/comments) — the same *content* the
//! validated prototype's `ast` pass produced. A tree-sitter upgrade (better
//! extraction + the repo-map call graph) slots in behind [`structural_doc`].
//!
//! The embedder is injected (`&dyn EmbeddingProvider`) so the flow unit-tests
//! with a fake; production passes the configured (cloud-default) provider, and
//! its `signature()` becomes the blob cache's `model` key.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::openhuman::embeddings::EmbeddingProvider;

use super::store::CodegraphStore;

const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "rb", "c", "cc", "cpp", "h", "hpp", "cs",
    "php", "kt", "swift", "scala", "sh",
];
const MAX_FILE_BYTES: u64 = 100_000;
const MAX_CALLS: usize = 200;
/// Structural docs embedded per provider call. One call per file would be one
/// network round-trip per file against a cloud embedder; batching collapses a
/// repo into a handful of calls.
const EMBED_BATCH: usize = 128;

/// Cache `model` key for a lexical-only (BM25, no embedding) index. Kept
/// separate from any embedder signature so a later dense pass indexes fresh
/// under its own key rather than colliding with these embedding-less rows.
pub const LEXICAL_MODEL: &str = "codegraph:lexical:v1";

/// What to build for a `(repo, ref)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    /// BM25 tokens only — no embedding calls. Cheap; enough for small repos
    /// where recall saturates anyway.
    Lexical,
    /// Structural-aug dense vectors + BM25 tokens — the full seed. Worth its
    /// embedding cost on larger repos.
    Dense,
}

impl IndexMode {
    /// The blob-cache `model` key this mode writes/reads under.
    pub fn model_key(self, embedder: &dyn EmbeddingProvider) -> String {
        match self {
            IndexMode::Lexical => LEXICAL_MODEL.to_string(),
            IndexMode::Dense => embedder.signature(),
        }
    }
}

/// Count tracked code files at the checkout — the cheap signal (`git ls-files`,
/// no reads/embeds) used to choose [`IndexMode`] before indexing.
pub fn count_code_files(repo_dir: &Path) -> Result<usize> {
    Ok(tree_blobs(repo_dir)?.len())
}

/// Per-index outcome. On a branch switch, `computed` is just the changed blobs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexReport {
    pub repo_id: String,
    pub git_ref: String,
    pub files: usize,
    pub computed: usize,
    pub cached: usize,
    pub skipped: usize,
}

fn git(repo_dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("git {args:?}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Branch name if on a branch, else the short commit SHA (detached).
pub fn current_ref(repo_dir: &Path) -> Result<String> {
    if let Ok(s) = git(repo_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        let s = s.trim();
        if !s.is_empty() {
            return Ok(s.to_string());
        }
    }
    Ok(git(repo_dir, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// `(path, blob_sha)` for tracked code files at the current checkout.
fn tree_blobs(repo_dir: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for line in git(repo_dir, &["ls-files", "-s"])?.lines() {
        // `<mode> <sha> <stage>\t<path>`
        let (meta, path) = match line.split_once('\t') {
            Some(p) => p,
            None => continue,
        };
        let sha = match meta.split_whitespace().nth(1) {
            Some(s) => s,
            None => continue,
        };
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if CODE_EXTS.contains(&ext) {
            out.push((path.to_string(), sha.to_string()));
        }
    }
    Ok(out)
}

/// Lexical tokens with identifier splitting (camelCase / snake_case), so the
/// BM25 arm matches `__floordiv__` and `floordiv`/`floor`/`div` alike.
pub fn code_tokens(text: &str) -> Vec<String> {
    let mut toks = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let low = raw.to_ascii_lowercase();
        toks.push(low.clone());
        // split camelCase / snake (already split on non-alnum) into sub-words
        let mut cur = String::new();
        let mut prev_lower = false;
        for ch in raw.chars() {
            if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
                toks.push(cur.to_ascii_lowercase());
                cur.clear();
            }
            cur.push(ch);
            prev_lower = ch.is_ascii_lowercase();
        }
        let sub = cur.to_ascii_lowercase();
        if !sub.is_empty() && sub != low {
            toks.push(sub);
        }
    }
    toks
}

/// Heuristic, content-only "intent" text: definition signatures + imports +
/// called-symbol identifiers + leading doc/comment lines. Path is excluded so
/// the result is purely content-derived (cacheable by blob SHA).
pub fn structural_doc(text: &str) -> String {
    let mut sigs = Vec::new();
    let mut imports = Vec::new();
    let mut docs = Vec::new();
    let mut calls: Vec<String> = Vec::new();
    let mut seen_calls = std::collections::HashSet::new();

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lead = t.split_whitespace().next().unwrap_or("");
        match lead {
            // definition keywords across the supported languages
            "fn" | "def" | "class" | "struct" | "impl" | "trait" | "enum" | "interface"
            | "type" | "func" | "function" | "module" | "public" | "private" | "protected"
            | "pub" | "async" | "export" | "const" => {
                sigs.push(t.trim_end_matches('{').trim().to_string());
            }
            "import" | "use" | "from" | "require" | "#include" | "package" => {
                imports.push(t.to_string());
            }
            _ => {}
        }
        if t.starts_with("//")
            || t.starts_with("///")
            || t.starts_with('#')
            || t.starts_with('*')
            || t.starts_with("\"\"\"")
        {
            docs.push(t.trim_start_matches(['/', '#', '*', ' ', '"']).to_string());
        }
        // naive call extraction: `ident(`
        for (i, _) in line.match_indices('(') {
            let prefix = &line[..i];
            let name: String = prefix
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if name.len() >= 2 && seen_calls.insert(name.clone()) && calls.len() < MAX_CALLS {
                calls.push(name);
            }
        }
    }

    let mut parts = Vec::new();
    if !sigs.is_empty() {
        parts.push(format!("symbols: {}", sigs.join(" ")));
    }
    if !imports.is_empty() {
        parts.push(format!("imports: {}", imports.join(" ")));
    }
    if !calls.is_empty() {
        parts.push(format!("calls: {}", calls.join(" ")));
    }
    if !docs.is_empty() {
        parts.push(format!("docs: {}", docs.join(" ")));
    }
    parts.join("\n")
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// (Re)index the checkout at `repo_dir` under `(repo_id, ref)`. Only blobs not
/// already cached for this `mode`'s key are read + (in `Dense`) embedded; the
/// rest are cache hits. Then the ref's manifest is rewritten to the current
/// tree. In `Lexical` mode no embedder call is made — tokens only.
pub async fn index_ref(
    store: &mut CodegraphStore,
    repo_id: &str,
    repo_dir: &Path,
    git_ref: Option<&str>,
    embedder: &dyn EmbeddingProvider,
    mode: IndexMode,
) -> Result<IndexReport> {
    let git_ref = match git_ref {
        Some(r) => r.to_string(),
        None => current_ref(repo_dir)?,
    };
    let model = mode.model_key(embedder);
    let blobs = tree_blobs(repo_dir)?;
    let (mut cached, mut skipped) = (0usize, 0usize);

    // Phase 1 — read + extract every *uncached, unique* blob. No DB writes and
    // no embedding yet, so phases 2 and 3 can batch both. A content SHA seen
    // twice in the tree (identical file) is extracted once.
    let mut seen = std::collections::HashSet::new();
    let mut pend_sha: Vec<String> = Vec::new();
    let mut pend_tokens: Vec<Vec<String>> = Vec::new();
    let mut pend_docs: Vec<String> = Vec::new();
    for (path, sha) in &blobs {
        if !seen.insert(sha.clone()) || store.has_blob(sha, &model)? {
            cached += 1;
            continue;
        }
        let full = repo_dir.join(path);
        match std::fs::metadata(&full) {
            Ok(m) if m.len() > MAX_FILE_BYTES => {
                skipped += 1;
                continue;
            }
            Err(_) => {
                skipped += 1;
                continue;
            }
            _ => {}
        }
        let text = match std::fs::read_to_string(&full) {
            Ok(t) => t,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let tokens = code_tokens(&text);
        if mode == IndexMode::Dense {
            // A file with no extractable structure (empty `__init__.py`, a data
            // file, `x = 1`) yields an empty structural doc. Embedders reject
            // empty input (the cloud backend 400s the whole batch), so fall
            // back to the lexical tokens — still content-derived, cacheable by
            // blob SHA. (Skipped entirely in Lexical mode — no embedding.)
            let doc = structural_doc(&text);
            let doc = if doc.trim().is_empty() {
                if tokens.is_empty() {
                    "(no extractable content)".to_string()
                } else {
                    tokens.join(" ")
                }
            } else {
                doc
            };
            pend_docs.push(doc);
        }
        pend_tokens.push(tokens);
        pend_sha.push(sha.clone());
    }

    // Phase 2 — produce a vector per pending blob. Lexical: empty vectors (no
    // embedder call). Dense: embed the structural docs in batches (few
    // round-trips, not one per file), L2-normalising each.
    let mut embs: Vec<Vec<f32>> = Vec::with_capacity(pend_sha.len());
    match mode {
        IndexMode::Lexical => embs.resize(pend_sha.len(), Vec::new()),
        IndexMode::Dense => {
            for chunk in pend_docs.chunks(EMBED_BATCH) {
                let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
                let out = embedder
                    .embed(&refs)
                    .await
                    .context("codegraph: embed structural docs")?;
                if out.len() != chunk.len() {
                    anyhow::bail!(
                        "codegraph: embedder returned {} vectors for {} inputs",
                        out.len(),
                        chunk.len()
                    );
                }
                for mut v in out {
                    l2_normalize(&mut v);
                    embs.push(v);
                }
            }
        }
    }

    // Phase 3 — persist the whole batch in one transaction, then rewrite the
    // ref's manifest.
    let computed = pend_sha.len();
    let entries: Vec<(String, Vec<String>, Vec<f32>)> = pend_sha
        .into_iter()
        .zip(pend_tokens)
        .zip(embs)
        .map(|((sha, tokens), emb)| (sha, tokens, emb))
        .collect();
    store.put_blobs(&model, &entries)?;
    store.set_manifest(repo_id, &git_ref, &blobs)?;

    Ok(IndexReport {
        repo_id: repo_id.to_string(),
        git_ref,
        files: blobs.len(),
        computed,
        cached,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tempfile::TempDir;

    #[test]
    fn code_tokens_splits_identifiers() {
        let t = code_tokens("def __floordiv__(self): TimedeltaIndex");
        assert!(t.contains(&"floordiv".to_string()));
        assert!(t.contains(&"timedelta".to_string()) || t.contains(&"timedeltaindex".to_string()));
    }

    #[test]
    fn structural_doc_pulls_signatures_imports_calls() {
        let src = "import os\nfn reconcile(charge):\n    return backoff(charge)\n";
        let d = structural_doc(src);
        assert!(d.contains("imports:") && d.contains("import os"));
        assert!(d.contains("symbols:") && d.contains("reconcile"));
        assert!(d.contains("calls:") && d.contains("backoff"));
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
            // deterministic non-zero vector per input (length-based, just needs to be stable)
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32 + 1.0, 1.0, 0.5])
                .collect())
        }
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[tokio::test]
    async fn index_ref_is_content_addressed_and_incremental() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.rs"), "fn reconcile() { backoff(); }\n").unwrap();
        std::fs::write(repo.join("readme.md"), "not code\n").unwrap(); // non-code ext → ignored
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();
        let emb = FakeEmbedder;

        let r1 = index_ref(&mut store, "r", &repo, Some("main"), &emb, IndexMode::Dense)
            .await
            .unwrap();
        assert_eq!(r1.files, 1, "only the .rs file is indexed");
        assert_eq!(r1.computed, 1);
        assert_eq!(r1.cached, 0);

        // Re-index unchanged tree → all cache hits, nothing re-embedded.
        let r2 = index_ref(&mut store, "r", &repo, Some("main"), &emb, IndexMode::Dense)
            .await
            .unwrap();
        assert_eq!(r2.computed, 0);
        assert_eq!(r2.cached, 1);

        // The blob hydrates with tokens + a normalized embedding.
        let hits = store.hydrate("r", "main", &emb.signature()).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].tokens.contains(&"reconcile".to_string()));
        let norm: f32 = hits[0].emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "embedding is L2-normalized");
    }

    /// An embedder that errors on empty input, like the real cloud backend
    /// (which 400s "input must be a non-empty string"). Guards the fallback.
    struct StrictEmbedder;
    #[async_trait]
    impl EmbeddingProvider for StrictEmbedder {
        fn name(&self) -> &str {
            "strict"
        }
        fn model_id(&self) -> &str {
            "strict-1"
        }
        fn dimensions(&self) -> usize {
            2
        }
        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            if texts.iter().any(|t| t.trim().is_empty()) {
                anyhow::bail!("input must be a non-empty string");
            }
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32 + 1.0, 1.0])
                .collect())
        }
    }

    #[tokio::test]
    async fn index_ref_never_embeds_empty_doc() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        // Structure-less files: empty, and a bare assignment (no def/import/call/doc).
        std::fs::write(repo.join("__init__.py"), "").unwrap();
        std::fs::write(repo.join("data.py"), "x = 1\n").unwrap();
        std::fs::write(repo.join("ok.py"), "def go():\n    run()\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();
        // Must NOT bail with the empty-input error: the fallback keeps every
        // embed input non-empty even for files with no extractable structure.
        let rep = index_ref(
            &mut store,
            "r",
            &repo,
            Some("main"),
            &StrictEmbedder,
            IndexMode::Dense,
        )
        .await
        .expect("index_ref tolerates structure-less files");
        assert_eq!(rep.computed, 3, "all three files embedded + stored");
    }

    /// Embedder that fails if called at all — proves the lexical path embeds nothing.
    struct NoEmbed;
    #[async_trait]
    impl EmbeddingProvider for NoEmbed {
        fn name(&self) -> &str {
            "noembed"
        }
        fn model_id(&self) -> &str {
            "noembed-1"
        }
        fn dimensions(&self) -> usize {
            2
        }
        async fn embed(&self, _t: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedder must not be called in lexical mode")
        }
    }

    #[tokio::test]
    async fn lexical_mode_indexes_and_searches_without_embedding() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("auth.rs"), "fn login() { session(); token(); }\n").unwrap();
        std::fs::write(repo.join("retry.rs"), "fn reconcile() { backoff(); }\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();
        // Lexical index makes no embedder call (NoEmbed would bail) …
        let rep = index_ref(
            &mut store,
            "r",
            &repo,
            Some("main"),
            &NoEmbed,
            IndexMode::Lexical,
        )
        .await
        .expect("lexical index never embeds");
        assert_eq!(rep.computed, 2);

        // … and lexical search is BM25-only — still no embedder call — yet ranks.
        let out = crate::openhuman::codegraph::search_ref(
            &mut store,
            "r",
            "main",
            "reconcile backoff",
            &NoEmbed,
            5,
        )
        .await
        .expect("lexical search never embeds");
        assert!(matches!(
            out.coverage,
            crate::openhuman::codegraph::Coverage::Full
        ));
        assert_eq!(
            out.hits.first().map(String::as_str),
            Some("retry.rs"),
            "BM25 ranks retry.rs first for 'reconcile backoff'"
        );
    }

    // ---- manual indexing benchmark -------------------------------------
    // A zero-latency embedder returning realistically-sized (default 1024-d)
    // vectors, with cumulative embed-time accounting so the harness can
    // subtract it and report *pure engine* throughput (extract + tokenize +
    // SQLite + manifest). Real cloud embedding latency adds on top of that.
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    struct BenchEmbedder {
        dim: usize,
        embed_nanos: Arc<AtomicU64>,
        invocations: Arc<AtomicU64>,
        docs: Arc<AtomicU64>,
    }
    #[async_trait]
    impl EmbeddingProvider for BenchEmbedder {
        fn name(&self) -> &str {
            "bench"
        }
        fn model_id(&self) -> &str {
            "bench-vec"
        }
        fn dimensions(&self) -> usize {
            self.dim
        }
        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            let t = std::time::Instant::now();
            let out: Vec<Vec<f32>> = texts
                .iter()
                .map(|s| {
                    // cheap, deterministic, non-degenerate vector of the real size
                    let mut v = vec![0.0f32; self.dim];
                    v[0] = s.len() as f32 + 1.0;
                    if self.dim > 1 {
                        v[1] = 1.0;
                    }
                    v
                })
                .collect();
            self.embed_nanos
                .fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            self.invocations.fetch_add(1, Ordering::Relaxed);
            self.docs.fetch_add(texts.len() as u64, Ordering::Relaxed);
            Ok(out)
        }
    }

    #[tokio::test]
    #[ignore = "manual benchmark: CODEGRAPH_BENCH_REPO=/path cargo test ... -- --ignored --nocapture"]
    async fn bench_index_speed() {
        let repo = match std::env::var("CODEGRAPH_BENCH_REPO") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => {
                eprintln!("bench_index_speed: set CODEGRAPH_BENCH_REPO=/path/to/git/repo");
                return;
            }
        };
        let dim: usize = std::env::var("CODEGRAPH_BENCH_DIM")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1024);

        let tmp = TempDir::new().unwrap();
        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();
        let embed_nanos = Arc::new(AtomicU64::new(0));
        let invocations = Arc::new(AtomicU64::new(0));
        let docs = Arc::new(AtomicU64::new(0));
        let emb = BenchEmbedder {
            dim,
            embed_nanos: embed_nanos.clone(),
            invocations: invocations.clone(),
            docs: docs.clone(),
        };

        // COLD — nothing cached, every blob is read + extracted + embedded + stored.
        let t0 = std::time::Instant::now();
        let cold = index_ref(&mut store, "bench", &repo, None, &emb, IndexMode::Dense)
            .await
            .unwrap();
        let cold_ms = t0.elapsed().as_secs_f64() * 1e3;
        let embed_ms = embed_nanos.load(Ordering::Relaxed) as f64 / 1e6;
        let engine_ms = (cold_ms - embed_ms).max(0.0);
        let n = cold.computed.max(1) as f64;

        // WARM — re-index the same tree: content-addressed → all cache hits.
        let t1 = std::time::Instant::now();
        let warm = index_ref(&mut store, "bench", &repo, None, &emb, IndexMode::Dense)
            .await
            .unwrap();
        let warm_ms = t1.elapsed().as_secs_f64() * 1e3;

        eprintln!("\n==== codegraph index bench =====================================");
        eprintln!("repo            : {}", repo.display());
        eprintln!("embed dim       : {dim}  (zero-latency fake embedder)");
        eprintln!(
            "files (tracked) : {}  computed={} cached={} skipped={}",
            cold.files, cold.computed, cold.cached, cold.skipped
        );
        eprintln!("-- COLD (full index) -------------------------------------------");
        eprintln!("  wall total    : {:>8.1} ms", cold_ms);
        eprintln!(
            "  fake embed    : {:>8.1} ms  ({:.1}% — replaced by real cloud latency in prod)",
            embed_ms,
            100.0 * embed_ms / cold_ms.max(1e-9)
        );
        eprintln!(
            "  ENGINE only   : {:>8.1} ms  → {:>7.0} files/s  ({:.3} ms/file)",
            engine_ms,
            n / (engine_ms / 1e3).max(1e-9),
            engine_ms / n
        );
        eprintln!(
            "  embed         : {} call(s) for {} docs  (batched ≤{}/call)",
            invocations.load(Ordering::Relaxed),
            docs.load(Ordering::Relaxed),
            EMBED_BATCH
        );
        eprintln!("-- WARM (content-addressed re-index, all cache hits) -----------");
        eprintln!(
            "  wall total    : {:>8.1} ms  → {:>7.0} files/s  ({:.4} ms/file)  cached={}",
            warm_ms,
            warm.files as f64 / (warm_ms / 1e3).max(1e-9),
            warm_ms / warm.files.max(1) as f64,
            warm.cached
        );
        eprintln!("================================================================\n");
    }

    /// Live probe — build the *real* provider from the workspace config and
    /// embed one string. Confirms the cloud session JWT + backend are reachable
    /// before attempting a full real-embedding index. A `401`/expired session
    /// prints `EMBED FAILED` rather than panicking.
    ///
    ///   OPENHUMAN_WORKSPACE=/path OPENHUMAN_KEYRING_BACKEND=file \
    ///     cargo test --lib codegraph::index::tests::cloud_embed_probe -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "live: needs OPENHUMAN_WORKSPACE + a valid backend session"]
    async fn cloud_embed_probe() {
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .expect("load config");
        let provider = crate::openhuman::embeddings::provider_from_config(&config)
            .expect("build embedding provider");
        eprintln!(
            "\n==== cloud embed probe ====\nprovider={} model={} dims={} sig={}",
            provider.name(),
            provider.model_id(),
            provider.dimensions(),
            provider.signature(),
        );
        let t = std::time::Instant::now();
        match provider.embed(&["hello world from codegraph"]).await {
            Ok(vs) => {
                let v = vs.first().map(Vec::as_slice).unwrap_or(&[]);
                eprintln!(
                    "OK: {} vector(s), dim={}, first5={:?}  ({:.0} ms)",
                    vs.len(),
                    v.len(),
                    &v[..v.len().min(5)],
                    t.elapsed().as_secs_f64() * 1e3
                );
            }
            Err(e) => eprintln!("EMBED FAILED: {e:#}"),
        }
        eprintln!("===========================\n");
    }

    /// Full real-embedding e2e: load the workspace config → build the cloud
    /// provider → `index_ref` a real repo → `search_ref`, asserting full
    /// coverage + non-empty hits and printing real wall-time (embedding
    /// included). Defaults to the small flask checkout (one embed batch);
    /// override with `CODEGRAPH_E2E_REPO` / `CODEGRAPH_E2E_QUERY`.
    ///
    ///   OPENHUMAN_WORKSPACE=/path OPENHUMAN_KEYRING_BACKEND=file \
    ///     cargo test --lib codegraph::index::tests::index_e2e_cloud -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "live: real cloud embeddings; needs OPENHUMAN_WORKSPACE + a valid session"]
    async fn index_e2e_cloud() {
        let repo = std::path::PathBuf::from(std::env::var("CODEGRAPH_E2E_REPO").unwrap_or_else(
            |_| {
                "/home/sanil/vezures/openhuman-cbmem-ab/bench/codebase-memory-ab/repos/pallets__flask"
                    .to_string()
            },
        ));
        if !repo.exists() {
            eprintln!("index_e2e_cloud: repo not found: {}", repo.display());
            return;
        }
        let query = std::env::var("CODEGRAPH_E2E_QUERY")
            .unwrap_or_else(|_| "register blueprint route url rule".to_string());

        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .expect("load config");
        let provider = crate::openhuman::embeddings::provider_from_config(&config)
            .expect("build embedding provider");

        let tmp = TempDir::new().unwrap();
        let mut store = CodegraphStore::open(&tmp.path().join("cg.db")).unwrap();

        let t0 = std::time::Instant::now();
        let rep = index_ref(
            &mut store,
            "e2e",
            &repo,
            None,
            provider.as_ref(),
            IndexMode::Dense,
        )
        .await
        .expect("index_ref");
        let index_ms = t0.elapsed().as_secs_f64() * 1e3;

        let t1 = std::time::Instant::now();
        let out = crate::openhuman::codegraph::search_ref(
            &mut store,
            "e2e",
            &rep.git_ref,
            &query,
            provider.as_ref(),
            10,
        )
        .await
        .expect("search_ref");
        let search_ms = t1.elapsed().as_secs_f64() * 1e3;

        eprintln!("\n==== codegraph e2e (REAL cloud embeddings) =====================");
        eprintln!("repo  : {}  ref={}", repo.display(), rep.git_ref);
        eprintln!(
            "index : files={} computed={} cached={} skipped={}  in {:.0} ms (embedding incl.)",
            rep.files, rep.computed, rep.cached, rep.skipped, index_ms
        );
        eprintln!("query : {query:?}");
        eprintln!(
            "search: coverage={:?} indexed={} total={}  in {:.0} ms",
            out.coverage, out.indexed, out.total, search_ms
        );
        eprintln!("top hits:");
        for (i, h) in out.hits.iter().take(10).enumerate() {
            eprintln!("  {}. {}", i + 1, h);
        }
        eprintln!("================================================================\n");

        assert!(rep.computed > 0, "indexed at least one blob");
        // Not None — we got real coverage. A clean small repo is Full; a large
        // repo with oversized/binary files skipped is legitimately Partial.
        assert!(
            !matches!(out.coverage, crate::openhuman::codegraph::Coverage::None),
            "search has at least partial coverage"
        );
        assert!(!out.hits.is_empty(), "search returned hits");
    }
}
