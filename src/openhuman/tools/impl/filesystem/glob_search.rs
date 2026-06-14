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
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use glob::Pattern;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
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
        "Find files matching a glob pattern (e.g. `src/**/*.rs`). Searches your working \
         directory (the action sandbox) by default; pass `path` to search another folder \
         you're allowed to read. Returns matching paths sorted by modification time \
         (newest first) — relative to the working directory, or absolute when outside it — \
         so each result can be passed straight to `file_read`/`grep`."
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

        log::debug!(
            "[tools:glob] search start: pattern='{pattern_str}' path='{search_path}' max_results={max_results}"
        );

        // Resolve + authorize the search root through the readers' validation
        // seam. A disallowed or missing root yields a clear, path-naming error
        // instead of the opaque "No such file or directory" the old workspace_dir
        // root produced (#3357).
        let base = match self.security.validate_path(search_path).await {
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
            self.security.action_dir.display()
        );

        // Canonical action sandbox, used to decide whether a hit is rendered
        // relative (inside the sandbox → directly file_read-able by relative
        // path) or absolute (in a granted trusted root → file_read-able as-is).
        // `base` is already canonical (validate_path canonicalizes), so if this
        // fallback fires the two roots become asymmetric and strip_prefix below
        // may miss for an in-sandbox hit, rendering it absolute. Harmless: the
        // absolute path is still readable and the per-hit filter still applies.
        let action_root = tokio::fs::canonicalize(&self.security.action_dir)
            .await
            .unwrap_or_else(|e| {
                log::trace!(
                    "[tools:glob] action_dir canonicalize fallback: path='{}' error={e}",
                    self.security.action_dir.display()
                );
                self.security.action_dir.clone()
            });

        let security = self.security.clone();
        let result = tokio::task::spawn_blocking(move || {
            collect_matches(&base, &action_root, &security, &pattern, max_results)
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
    hits.sort_by(|a, b| b.0.cmp(&a.0));
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
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot};

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            action_dir: workspace.clone(),
            workspace_dir: workspace.clone(),
            // Mirror the production constructor, which registers the action
            // sandbox as a ReadWrite trusted root so validate_path accepts it
            // even under workspace_only.
            trusted_roots: vec![TrustedRoot {
                path: workspace.to_string_lossy().to_string(),
                access: TrustedAccess::ReadWrite,
            }],
            ..SecurityPolicy::default()
        })
    }

    /// Policy with a distinct action sandbox and internal workspace — the real
    /// production shape, and the configuration that surfaced #3357.
    fn test_security_split(
        action_dir: std::path::PathBuf,
        workspace_dir: std::path::PathBuf,
        extra_roots: Vec<std::path::PathBuf>,
    ) -> Arc<SecurityPolicy> {
        let mut roots = vec![TrustedRoot {
            path: action_dir.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        }];
        roots.extend(extra_roots.into_iter().map(|p| TrustedRoot {
            path: p.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        }));
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            action_dir,
            workspace_dir,
            trusted_roots: roots,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn glob_name() {
        let tool = GlobTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "glob");
    }

    #[tokio::test]
    async fn glob_matches_extension() {
        let dir = std::env::temp_dir().join("openhuman_test_glob_ext");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("src/sub"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("src/a.rs"), "// a")
            .await
            .unwrap();
        tokio::fs::write(dir.join("src/sub/b.rs"), "// b")
            .await
            .unwrap();
        tokio::fs::write(dir.join("src/c.txt"), "c").await.unwrap();

        let tool = GlobTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"pattern": "**/*.rs"})).await.unwrap();
        assert!(!result.is_error);
        let output = result.output();
        assert!(output.contains("src/a.rs"));
        assert!(output.contains("src/sub/b.rs"));
        assert!(!output.contains("c.txt"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn glob_invalid_pattern() {
        let dir = std::env::temp_dir().join("openhuman_test_glob_invalid");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let tool = GlobTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"pattern": "**["})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Invalid glob"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn glob_skips_node_modules() {
        let dir = std::env::temp_dir().join("openhuman_test_glob_skip");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("node_modules"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("node_modules/lib.js"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.join("app.js"), "").await.unwrap();

        let tool = GlobTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"pattern": "**/*.js"})).await.unwrap();
        let output = result.output();
        assert!(output.contains("app.js"));
        assert!(!output.contains("node_modules"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// Regression for #3357: glob roots at action_dir (so its hits are readable),
    /// and never surfaces files living under the internal workspace_dir.
    #[tokio::test]
    async fn glob_roots_at_action_dir_and_excludes_workspace() {
        let root = std::env::temp_dir().join("openhuman_test_glob_split");
        let action = root.join("action");
        let workspace = root.join("workspace");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&action).await.unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(action.join("keep.txt"), "keep")
            .await
            .unwrap();
        tokio::fs::write(workspace.join("secret.txt"), "secret")
            .await
            .unwrap();

        let security = test_security_split(action.clone(), workspace.clone(), vec![]);
        let tool = GlobTool::new(security.clone());
        let result = tool.execute(json!({"pattern": "**/*.txt"})).await.unwrap();
        assert!(!result.is_error, "{}", result.output());
        let output = result.output();
        // Action-sandbox file is found, rendered relative...
        assert!(output.contains("keep.txt"), "missing keep.txt: {output}");
        // ...and the internal workspace file is NOT enumerated.
        assert!(
            !output.contains("secret.txt"),
            "leaked workspace file: {output}"
        );

        // The glob hit must be directly readable by the reader tools.
        assert!(
            security.validate_path("keep.txt").await.is_ok(),
            "glob hit not resolvable by validate_path"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// "As wide as the readers": glob can search any granted trusted root via
    /// the `path` arg, returning absolute paths that the readers accept as-is.
    #[tokio::test]
    async fn glob_searches_named_trusted_root() {
        let root = std::env::temp_dir().join("openhuman_test_glob_trusted");
        let action = root.join("action");
        let workspace = root.join("workspace");
        let granted = root.join("granted");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&action).await.unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&granted).await.unwrap();
        tokio::fs::write(granted.join("data.csv"), "x")
            .await
            .unwrap();

        let security =
            test_security_split(action.clone(), workspace.clone(), vec![granted.clone()]);
        let tool = GlobTool::new(security.clone());
        let granted_abs = granted.to_string_lossy().to_string();
        let result = tool
            .execute(json!({"pattern": "**/*.csv", "path": granted_abs}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let output = result.output();
        assert!(output.contains("data.csv"), "missing data.csv: {output}");
        // Rendered absolute (outside the action sandbox).
        assert!(
            output.contains(&granted.to_string_lossy().to_string()),
            "expected absolute path: {output}"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// A search root the policy disallows yields a clear error, not ENOENT.
    #[tokio::test]
    async fn glob_rejects_disallowed_search_path() {
        let root = std::env::temp_dir().join("openhuman_test_glob_reject");
        let action = root.join("action");
        let workspace = root.join("workspace");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&action).await.unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();

        let security = test_security_split(action.clone(), workspace.clone(), vec![]);
        let tool = GlobTool::new(security);
        // Absolute path outside any trusted root, under workspace_only.
        let result = tool
            .execute(json!({"pattern": "**/*", "path": "/etc"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.output().contains("not accessible"),
            "{}",
            result.output()
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// Security backstop: a symlink inside the sandbox pointing OUTSIDE it must
    /// never leak the target. Two layers cover this and this test exercises
    /// both: the walk runs with `follow_links(false)`, so the symlink entry is
    /// dropped by the `is_file()` gate (a symlink is not a regular file) and is
    /// never descended; and `is_path_string_allowed` is the fail-closed per-hit
    /// backstop — assert directly that it rejects the escape's resolved string,
    /// since that is the check `collect_matches` leans on if the walk gate is
    /// ever bypassed. A legitimate in-sandbox file is still found.
    #[cfg(unix)]
    #[tokio::test]
    async fn glob_does_not_leak_symlink_escape() {
        let root = std::env::temp_dir().join("openhuman_test_glob_symlink");
        let action = root.join("action");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(action.join("sub")).await.unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();
        tokio::fs::write(action.join("sub/ok.txt"), "ok")
            .await
            .unwrap();
        tokio::fs::write(outside.join("secret.txt"), "secret")
            .await
            .unwrap();
        // Symlink inside the sandbox pointing at the outside (untrusted) tree.
        std::os::unix::fs::symlink(&outside, action.join("escape")).unwrap();

        // `outside` is deliberately NOT registered as a trusted root.
        let security = test_security_split(action.clone(), workspace.clone(), vec![]);
        let tool = GlobTool::new(security.clone());
        let result = tool.execute(json!({"pattern": "**/*.txt"})).await.unwrap();
        assert!(!result.is_error, "{}", result.output());
        let output = result.output();
        // The in-sandbox file is found, the escape target is not enumerated.
        assert!(
            output.contains("ok.txt"),
            "missing in-sandbox file: {output}"
        );
        assert!(
            !output.contains("secret"),
            "leaked symlink-escape target: {output}"
        );
        // The fail-closed backstop, exercised directly: the resolved escape
        // path is rejected as a string regardless of how it was reached.
        let escaped = outside.join("secret.txt").to_string_lossy().to_string();
        assert!(
            !security.is_path_string_allowed(&escaped),
            "policy must reject the escape target string: {escaped}"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// A `path` that resolves to a *file* (not a directory) yields a clear
    /// "not a directory" error rather than a misleading "0 match(es)".
    #[tokio::test]
    async fn glob_rejects_file_search_path() {
        let dir = std::env::temp_dir().join("openhuman_test_glob_file_root");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("file.txt"), "x").await.unwrap();

        let tool = GlobTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"pattern": "**/*", "path": "file.txt"}))
            .await
            .unwrap();
        assert!(result.is_error, "{}", result.output());
        assert!(
            result.output().contains("not a directory"),
            "{}",
            result.output()
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
