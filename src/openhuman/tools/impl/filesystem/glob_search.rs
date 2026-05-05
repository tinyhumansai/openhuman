//! `glob` — find files by glob pattern.
//!
//! Coding-harness baseline tool (issue #1205): pure file discovery
//! by pattern (e.g. `src/**/*.rs`). Path traversal is blocked the same
//! way as `file_read`.

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
        "Find files matching a glob pattern (e.g. `src/**/*.rs`). Returns matching paths \
         relative to the workspace, sorted by modification time (newest first)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. `**/*.rs` or `src/**/*.{ts,tsx}` (single brace expansion not supported — list patterns separately)."
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

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pattern_str = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;
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

        let workspace = self.security.workspace_dir.clone();
        let result =
            tokio::task::spawn_blocking(move || collect_matches(&workspace, &pattern, max_results))
                .await
                .map_err(|e| anyhow::anyhow!("scan task failed: {e}"))?;

        let (paths, truncated) = result;
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

fn collect_matches(workspace: &Path, pattern: &Pattern, max_results: usize) -> (Vec<String>, bool) {
    let mut hits: Vec<(std::time::SystemTime, String)> = Vec::new();

    for entry in WalkDir::new(workspace)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e.file_name().to_string_lossy().as_ref()))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = match entry.path().strip_prefix(workspace) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !pattern.matches(&rel_str) {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        hits.push((mtime, rel_str));
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
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
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
}
