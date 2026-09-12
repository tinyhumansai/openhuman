//! `glob` — find files by glob pattern.
//!
//! Coding-harness baseline tool (issue #1205): pure file discovery
//! by pattern (e.g. `src/**/*.rs`).
//!
//! ## Path-root contract (issue #3357)
//!
//! `glob` resolves its search root through the **same** seam the reader tools
//! use — [`SecurityPolicy::validate_path`] — and filters every hit through
//! [`SecurityPolicy::is_path_string_allowed`]. This guarantees the core
//! invariant a discovery tool must hold: **every path `glob` returns is a path
//! `file_read`/`grep`/`list` can open, and nothing else.**
//!
//! Previously `glob` walked `workspace_dir` (internal product state) and
//! returned paths relative to it, while the readers resolve relative paths
//! against `action_dir`. The two roots normally differ, so a path `glob`
//! returned was meaningless to the readers — every follow-up read failed with
//! "No such file or directory (os error 2)" (#3357). Walking `workspace_dir`
//! also let the agent enumerate internal state (memory DBs, sessions, tokens).
//!
//! The optional `path` argument lets the agent search any location it is
//! allowed to read (the action sandbox by default, or a granted `trusted_root`)
//! — it is resolved through `validate_path`, so it can never widen access
//! beyond what the readers already permit.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use glob::Pattern;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tinytools::ToolRunContext;
use walkdir::WalkDir;

const DEFAULT_MAX_RESULTS: usize = 500;

pub struct GlobTool {
    security: Arc<SecurityPolicy>,
}

impl GlobTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. `src/**/*.rs`), searching the action sandbox unless `path` says otherwise. Returns paths newest-first, ready to pass straight to `file_read` / `grep`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. `**/*.rs` or `src/**/*.{ts,tsx}` (single brace expansion not supported — list patterns separately). Matched against paths relative to the search directory."
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search within. Defaults to the working directory (action sandbox). Must be a location you're allowed to read; relative paths resolve against the working directory."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Cap on returned paths (default 500).",
                    "minimum": 1
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    /// Pure read — safe to fan out across parallel `glob` calls.
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, context).await
    }
}

impl GlobTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let pattern_str = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;
        // Default the search root to "." — which `validate_path` resolves to the
        // action sandbox (action_dir), the same root file_read/grep/list use.
        let search_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".");
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1))
            .unwrap_or(DEFAULT_MAX_RESULTS);

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let pattern = match Pattern::new(pattern_str) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Invalid glob pattern: {e}"))),
        };

        let path_policy = super::security_for_tool_context(&self.security, context, "glob");

        log::debug!(
            "[tools:glob] search start: pattern='{pattern_str}' path='{search_path}' max_results={max_results}"
        );

        // Resolve + authorize the search root through the readers' validation
        // seam. A disallowed or missing root yields a clear, path-naming error
        // instead of the opaque "No such file or directory" the old workspace_dir
        // root produced (#3357).
        let base = match path_policy.validate_path(search_path).await {
            Ok(p) => p,
            Err(e) => {
                log::debug!("[tools:glob] search path rejected: path='{search_path}' error={e}");
                return Ok(ToolResult::error(format!(
                    "glob search path '{search_path}' is not accessible: {e}"
                )));
            }
        };
        // `validate_path` can authorize a *file* as well as a directory. Walking a
        // file root yields the file itself, whose path relative to `base` is empty,
        // so the pattern never matches and the agent gets a misleading "0 match(es)"
        // instead of a clear error. Reject non-directory roots explicitly.
        match tokio::fs::metadata(&base).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                log::debug!("[tools:glob] search path is not a directory: path='{search_path}'");
                return Ok(ToolResult::error(format!(
                    "glob search path '{search_path}' is not a directory"
                )));
            }
            Err(e) => {
                log::debug!(
                    "[tools:glob] search path not accessible: path='{search_path}' error={e}"
                );
                return Ok(ToolResult::error(format!(
                    "glob search path '{search_path}' is not accessible: {e}"
                )));
            }
        };
        log::debug!(
            "[tools:glob] resolved search root: '{}' (action_dir='{}')",
            base.display(),
            path_policy.action_dir.display()
        );

        // Canonical action sandbox, used to decide whether a hit is rendered
        // relative (inside the sandbox → directly file_read-able by relative
        // path) or absolute (in a granted trusted root → file_read-able as-is).
        // `base` is already canonical (validate_path canonicalizes), so if this
        // fallback fires the two roots become asymmetric and strip_prefix below
        // may miss for an in-sandbox hit, rendering it absolute. Harmless: the
        // absolute path is still readable and the per-hit filter still applies.
        let action_root = tokio::fs::canonicalize(&path_policy.action_dir)
            .await
            .unwrap_or_else(|e| {
                log::trace!(
                    "[tools:glob] action_dir canonicalize fallback: path='{}' error={e}",
                    path_policy.action_dir.display()
                );
                path_policy.action_dir.clone()
            });

        let result = tokio::task::spawn_blocking(move || {
            collect_matches(&base, &action_root, &path_policy, &pattern, max_results)
        })
        .await
        .map_err(|e| anyhow::anyhow!("scan task failed: {e}"))?;

        let (paths, truncated) = result;
        log::debug!(
            "[tools:glob] scan complete: {} match(es) truncated={}",
            paths.len(),
            truncated
        );
        let header = if truncated {
            format!("{} match(es) (truncated at {max_results})", paths.len())
        } else {
            format!("{} match(es)", paths.len())
        };

        let mut body = String::with_capacity(paths.len() * 32 + header.len() + 1);
        body.push_str(&header);
        for p in paths {
            body.push('\n');
            body.push_str(&p);
        }
        Ok(ToolResult::success(body))
    }
}

/// Walk `base`, returning glob matches as paths the reader tools can open.
///
/// - The pattern is matched against each file's path **relative to `base`**
///   (the searched directory), the conventional glob behavior.
/// - The returned string is **relative to `action_root`** when the hit lives
///   inside the action sandbox (so `file_read("src/a.rs")` works), otherwise
///   the **absolute** path (so `file_read` resolves it as-is via a trusted
///   root).
/// - Every hit is filtered through [`SecurityPolicy::is_path_string_allowed`]
///   on that exact returned string, so `glob` never surfaces a path the readers
///   would reject (internal state, forbidden trees, symlink escapes).
///
/// `max_results` bounds the returned list, not the walk: the full tree under
/// `base` is always traversed and only the output is truncated (pre-existing
/// behavior — the cap caps results, not work).
fn collect_matches(
    base: &Path,
    action_root: &Path,
    security: &SecurityPolicy,
    pattern: &Pattern,
    max_results: usize,
) -> (Vec<String>, bool) {
    let mut hits: Vec<(std::time::SystemTime, String)> = Vec::new();

    for entry in WalkDir::new(base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e.file_name().to_string_lossy().as_ref()))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();

        // Match the pattern against the path relative to the searched root.
        let rel_to_base = match abs.strip_prefix(base) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if !pattern.matches(&rel_to_base) {
            continue;
        }

        // Render the path the way the agent should hand it to file_read:
        // sandbox-relative when inside action_dir, absolute otherwise.
        let display = match abs.strip_prefix(action_root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => abs.to_string_lossy().to_string(),
        };

        // Fail-closed: only surface paths the readers would also accept.
        if !security.is_path_string_allowed(&display) {
            log::trace!("[tools:glob] path filtered by policy: '{display}'");
            continue;
        }

        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        hits.push((mtime, display));
    }

    // Newest first.
    hits.sort_by_key(|item| std::cmp::Reverse(item.0));
    let truncated = hits.len() > max_results;
    let paths: Vec<String> = hits.into_iter().take(max_results).map(|(_, p)| p).collect();
    (paths, truncated)
}

fn is_skipped(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | ".next" | "dist" | "build" | ".cache"
    )
}

#[cfg(test)]
#[path = "glob_search_tests.rs"]
mod tests;
