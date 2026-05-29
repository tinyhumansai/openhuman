//! Agent-facing codegraph tools: `codegraph_index` (start/refresh a repo's
//! index) and `codegraph_search` (the fused BM25 ∪ dense seed). Coding
//! subagents call these on a checked-out worktree; the embedder is the
//! configured (cloud-default) provider, and its `signature()` keys the cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use serde_json::Value;
use tracing::debug;

use crate::openhuman::codegraph::{
    count_code_files, current_ref, index_ref, search_ref, CodegraphStore, IndexMode,
};
use crate::openhuman::config::Config;
use crate::openhuman::embeddings;
use crate::openhuman::tools::traits::{Tool, ToolResult};

fn codegraph_db(workspace_dir: &Path) -> std::path::PathBuf {
    workspace_dir.join("codegraph").join("index.db")
}

/// File count at/above which auto-indexing builds the dense (embedding) index;
/// below it, BM25-only. Small repos saturate recall, so dense buys little there
/// while costing real embedding latency. Override with the env var.
fn dense_min_files() -> usize {
    std::env::var("OPENHUMAN_CODEGRAPH_DENSE_MIN_FILES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400)
}

/// Size-gated mode for `auto`: dense above the threshold, else lexical. The
/// count is cheap (`git ls-files`, no reads/embeds).
fn auto_mode(repo_dir: &Path) -> IndexMode {
    match count_code_files(repo_dir) {
        Ok(n) if n > dense_min_files() => IndexMode::Dense,
        _ => IndexMode::Lexical,
    }
}

/// Resolve an explicit `mode` arg (`auto`/`lexical`/`dense`) against the repo.
fn resolve_mode(arg: Option<&str>, repo_dir: &Path) -> IndexMode {
    match arg {
        Some("dense") => IndexMode::Dense,
        Some("lexical") => IndexMode::Lexical,
        _ => auto_mode(repo_dir),
    }
}

/// Stable per-repo key: the canonical worktree path (manifests are per
/// `(repo_id, ref)`; the blob cache is content-addressed so it's shared anyway).
fn repo_id(repo_dir: &Path) -> String {
    std::fs::canonicalize(repo_dir)
        .unwrap_or_else(|_| repo_dir.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

/// Resolve a caller-provided `path` against the workspace, refusing anything
/// outside it. Both the requested path and the workspace are canonicalized
/// (resolving `..`/symlinks), then `repo_dir` must live under the canonical
/// workspace — otherwise an agent could index/search arbitrary repos on disk.
fn resolve_repo_dir(path: &str, workspace_dir: &Path) -> anyhow::Result<PathBuf> {
    let repo_dir = std::fs::canonicalize(path)
        .with_context(|| format!("codegraph: cannot resolve repo path `{path}`"))?;
    let workspace = std::fs::canonicalize(workspace_dir).with_context(|| {
        format!(
            "codegraph: cannot resolve workspace dir `{}`",
            workspace_dir.display()
        )
    })?;
    if !repo_dir.starts_with(&workspace) {
        return Err(anyhow!(
            "codegraph: repo path `{}` is outside the workspace `{}`",
            repo_dir.display(),
            workspace.display()
        ));
    }
    Ok(repo_dir)
}

/// `codegraph_index { path, ref? }` — (re)index the worktree at `path` under its
/// current branch (or `ref`). Incremental: only changed blobs are embedded.
pub struct CodegraphIndexTool {
    config: Arc<Config>,
    workspace_dir: std::path::PathBuf,
}

impl CodegraphIndexTool {
    pub fn new(config: Arc<Config>, workspace_dir: std::path::PathBuf) -> Self {
        Self {
            config,
            workspace_dir,
        }
    }
}

#[async_trait]
impl Tool for CodegraphIndexTool {
    fn name(&self) -> &str {
        "codegraph_index"
    }

    fn description(&self) -> &str {
        "Index a checked-out repo for fast retrieval. Args: `path` (repo working dir, required), \
         `ref` (branch/commit; defaults to the current checkout), `mode` (`auto` (default) | `lexical` | `dense`). \
         `auto` builds BM25-only for small repos and adds dense embeddings above a file-count threshold. \
         Incremental and content-addressed — only changed files are (re)processed. \
         Returns {mode, files, computed, cached, skipped}."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Repo working directory to index."},
                "ref": {"type": "string", "description": "Branch/commit to index (defaults to current checkout)."},
                "mode": {"type": "string", "enum": ["auto", "lexical", "dense"], "description": "auto (size-gated, default), lexical (BM25 only), or dense (embeddings)."}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let path = match arg_str(&args, "path") {
            Some(p) => p,
            None => {
                debug!("[codegraph:index] missing `path` arg; returning error");
                return Ok(ToolResult::error(
                    "codegraph_index: `path` (repo working dir) is required",
                ));
            }
        };
        let repo_dir = resolve_repo_dir(path, &self.workspace_dir)?;
        let git_ref = match arg_str(&args, "ref") {
            Some(r) => r.to_string(),
            None => current_ref(&repo_dir)?,
        };
        let mode = resolve_mode(arg_str(&args, "mode"), &repo_dir);
        let rid = repo_id(&repo_dir);
        debug!(
            repo = %rid,
            git_ref = %git_ref,
            ?mode,
            "[codegraph:index] resolved mode; indexing"
        );
        let provider = match embeddings::provider_from_config(&self.config) {
            Ok(p) => p,
            Err(e) => {
                debug!(repo = %rid, error = %e, "[codegraph:index] provider error");
                return Err(e);
            }
        };
        let mut store = match CodegraphStore::open(&codegraph_db(&self.workspace_dir)) {
            Ok(s) => s,
            Err(e) => {
                debug!(repo = %rid, error = %e, "[codegraph:index] store open error");
                return Err(e);
            }
        };
        let report = match index_ref(
            &mut store,
            &rid,
            &repo_dir,
            Some(&git_ref),
            &*provider,
            mode,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(repo = %rid, git_ref = %git_ref, error = %e, "[codegraph:index] index_ref failed");
                return Err(e);
            }
        };
        debug!(
            repo = %rid,
            git_ref = %git_ref,
            ?mode,
            files = report.files,
            computed = report.computed,
            cached = report.cached,
            skipped = report.skipped,
            "[codegraph:index] success"
        );
        let out = serde_json::json!({
            "mode": if mode == IndexMode::Dense { "dense" } else { "lexical" },
            "files": report.files,
            "computed": report.computed,
            "cached": report.cached,
            "skipped": report.skipped,
        });
        Ok(ToolResult::success(serde_json::to_string_pretty(&out)?))
    }
}

/// `codegraph_search { query, path, ref?, k? }` — the seed: BM25 ∪ dense,
/// RRF-fused, with a `coverage` flag (`full`/`partial`/`none`). On `none`/`partial`
/// the agent should treat hits as hints and lean on grep.
pub struct CodegraphSearchTool {
    config: Arc<Config>,
    workspace_dir: std::path::PathBuf,
}

impl CodegraphSearchTool {
    pub fn new(config: Arc<Config>, workspace_dir: std::path::PathBuf) -> Self {
        Self {
            config,
            workspace_dir,
        }
    }
}

#[async_trait]
impl Tool for CodegraphSearchTool {
    fn name(&self) -> &str {
        "codegraph_search"
    }

    fn description(&self) -> &str {
        "Find the files most relevant to a query in a repo (lexical + semantic, fused). \
         Indexes the repo first if it hasn't been indexed yet (synchronous; BM25-only for small \
         repos, dense embeddings for larger ones). \
         Args: `query` (required), `path` (repo working dir, required), `ref` (defaults to current), \
         `k` (max hits, default 10). Returns {hits:[paths], coverage:full|partial|none, indexed, total}. \
         If coverage is not `full`, treat hits as hints and also use grep."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What to find (issue text / symbols)."},
                "path": {"type": "string", "description": "Repo working directory."},
                "ref": {"type": "string", "description": "Branch/commit (defaults to current checkout)."},
                "k": {"type": "integer", "description": "Max hits to return (default 10)."}
            },
            "required": ["query", "path"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = match arg_str(&args, "query") {
            Some(q) => q,
            None => {
                debug!("[codegraph:search] missing `query` arg; returning error");
                return Ok(ToolResult::error("codegraph_search: `query` is required"));
            }
        };
        let path = match arg_str(&args, "path") {
            Some(p) => p,
            None => {
                debug!("[codegraph:search] missing `path` arg; returning error");
                return Ok(ToolResult::error(
                    "codegraph_search: `path` (repo working dir) is required",
                ));
            }
        };
        let repo_dir = resolve_repo_dir(path, &self.workspace_dir)?;
        let git_ref = match arg_str(&args, "ref") {
            Some(r) => r.to_string(),
            None => current_ref(&repo_dir)?,
        };
        let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let rid = repo_id(&repo_dir);
        let provider = match embeddings::provider_from_config(&self.config) {
            Ok(p) => p,
            Err(e) => {
                debug!(repo = %rid, error = %e, "[codegraph:search] provider error");
                return Err(e);
            }
        };
        let mut store = match CodegraphStore::open(&codegraph_db(&self.workspace_dir)) {
            Ok(s) => s,
            Err(e) => {
                debug!(repo = %rid, error = %e, "[codegraph:search] store open error");
                return Err(e);
            }
        };
        // Index-first: if this (repo, ref) has never been indexed, build it now
        // (synchronously) so the search has something to hit. Mode is size-gated
        // — BM25-only for small repos, dense above the threshold.
        if store.manifest_size(&rid, &git_ref)? == 0 {
            let mode = auto_mode(&repo_dir);
            debug!(
                repo = %rid,
                git_ref = %git_ref,
                ?mode,
                "[codegraph:search] no manifest; auto-indexing before search"
            );
            if let Err(e) = index_ref(
                &mut store,
                &rid,
                &repo_dir,
                Some(&git_ref),
                &*provider,
                mode,
            )
            .await
            {
                debug!(repo = %rid, git_ref = %git_ref, error = %e, "[codegraph:search] auto-index failed");
                return Err(e);
            }
        }
        let outcome = match search_ref(&mut store, &rid, &git_ref, query, &*provider, k).await {
            Ok(o) => o,
            Err(e) => {
                debug!(repo = %rid, git_ref = %git_ref, error = %e, "[codegraph:search] search_ref failed");
                return Err(e);
            }
        };
        debug!(
            repo = %rid,
            git_ref = %git_ref,
            coverage = ?outcome.coverage,
            indexed = outcome.indexed,
            total = outcome.total,
            hits = outcome.hits.len(),
            "[codegraph:search] success"
        );
        Ok(ToolResult::success(serde_json::to_string_pretty(&outcome)?))
    }
}
