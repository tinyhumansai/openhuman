//! `grep` — regex search across files in the workspace.
//!
//! Coding-harness baseline tool (issue #1205): a first-class
//! file-navigation primitive that lets the agent search for a regex
//! across the workspace without falling through to `shell`. Uses the
//! same path-sandboxing + rate-limiting as `file_read`.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use walkdir::WalkDir;

const DEFAULT_MAX_MATCHES: usize = 200;
const MAX_LINE_BYTES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

pub struct GrepTool {
    security: Arc<SecurityPolicy>,
}

impl GrepTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents in the workspace with a regular expression. \
         Returns up to `max_matches` matches (default 200) as `path:line:text` lines."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression (Rust `regex` crate syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "Optional sub-path to restrict the search to (relative to workspace).",
                    "default": "."
                },
                "max_matches": {
                    "type": "integer",
                    "description": "Cap on matches returned (default 200).",
                    "minimum": 1
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "If true, compile the regex with the `i` flag.",
                    "default": false
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    /// Pure read — safe to fan out across parallel `grep` calls.
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;
        let sub_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let max_matches = args
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1))
            .unwrap_or(DEFAULT_MAX_MATCHES);
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.is_path_allowed(sub_path) {
            return Ok(ToolResult::error(format!(
                "Path not allowed by security policy: {sub_path}"
            )));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let regex = match build_regex(pattern, case_insensitive) {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Invalid regex: {e}"))),
        };

        let root = self.security.workspace_dir.join(sub_path);
        let resolved_root = match tokio::fs::canonicalize(&root).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Failed to resolve path: {e}"))),
        };
        if !self.security.is_resolved_path_allowed(&resolved_root) {
            return Ok(ToolResult::error(format!(
                "Resolved path escapes workspace: {}",
                resolved_root.display()
            )));
        }

        let workspace = self.security.workspace_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            scan_for_matches(&resolved_root, &workspace, &regex, max_matches)
        })
        .await
        .map_err(|e| anyhow::anyhow!("scan task failed: {e}"))?;

        let (matches, scanned, truncated) = result;
        let header = if truncated {
            format!(
                "{} match(es) (truncated at {max_matches}); scanned {scanned} file(s)",
                matches.len()
            )
        } else {
            format!("{} match(es); scanned {scanned} file(s)", matches.len())
        };

        let mut body = String::with_capacity(matches.len() * 80 + header.len() + 1);
        body.push_str(&header);
        for m in &matches {
            body.push('\n');
            body.push_str(m);
        }
        Ok(ToolResult::success(body))
    }
}

fn build_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, regex::Error> {
    if case_insensitive {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
    } else {
        Regex::new(pattern)
    }
}

fn scan_for_matches(
    root: &Path,
    workspace: &Path,
    regex: &Regex,
    max_matches: usize,
) -> (Vec<String>, usize, bool) {
    let mut matches: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;

    'outer: for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e.file_name().to_string_lossy().as_ref()))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        scanned += 1;
        let rel = path.strip_prefix(workspace).unwrap_or(path);
        for (lineno, line) in contents.lines().enumerate() {
            if regex.is_match(line) {
                // `MAX_LINE_BYTES` is a BYTE budget — use the byte-aware
                // helper. The earlier migration to `truncate_with_suffix`
                // mis-typed this as a char budget; for multi-byte text
                // (CJK / emoji) the rendered line could balloon to ~3×
                // the intended cap. Per CodeRabbit critical review on
                // PR #1549.
                let display_line =
                    crate::openhuman::util::truncate_at_byte_boundary(line, MAX_LINE_BYTES);
                matches.push(format!("{}:{}:{}", rel.display(), lineno + 1, display_line));
                if matches.len() >= max_matches {
                    truncated = true;
                    break 'outer;
                }
            }
        }
    }
    (matches, scanned, truncated)
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
    fn grep_name_and_schema() {
        let tool = GrepTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "grep");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("pattern")));
    }

    #[tokio::test]
    async fn grep_finds_matches() {
        let dir = std::env::temp_dir().join("openhuman_test_grep_finds");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "alpha\nbravo\ncharlie")
            .await
            .unwrap();
        tokio::fs::write(dir.join("b.txt"), "alpha2").await.unwrap();

        let tool = GrepTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"pattern": "^alpha"})).await.unwrap();
        assert!(!result.is_error);
        let output = result.output();
        assert!(output.contains("a.txt:1:alpha"));
        assert!(output.contains("b.txt:1:alpha2"));
        assert!(!output.contains("bravo"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn grep_invalid_regex() {
        let dir = std::env::temp_dir().join("openhuman_test_grep_invalid");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = GrepTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"pattern": "([unclosed"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Invalid regex"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn grep_case_insensitive() {
        let dir = std::env::temp_dir().join("openhuman_test_grep_ci");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("c.txt"), "Hello World")
            .await
            .unwrap();

        let tool = GrepTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"pattern": "hello", "case_insensitive": true}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("c.txt:1:Hello World"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn grep_blocks_path_traversal() {
        let tool = GrepTool::new(test_security(std::env::temp_dir()));
        let result = tool
            .execute(json!({"pattern": ".", "path": "../.."}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not allowed"));
    }

    #[tokio::test]
    async fn grep_skips_node_modules_and_git() {
        let dir = std::env::temp_dir().join("openhuman_test_grep_skip");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("node_modules"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(dir.join(".git")).await.unwrap();
        tokio::fs::write(dir.join("node_modules/x.txt"), "needle")
            .await
            .unwrap();
        tokio::fs::write(dir.join(".git/x.txt"), "needle")
            .await
            .unwrap();
        tokio::fs::write(dir.join("real.txt"), "needle")
            .await
            .unwrap();

        let tool = GrepTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"pattern": "needle"})).await.unwrap();
        assert!(!result.is_error);
        let output = result.output();
        assert!(output.contains("real.txt"));
        assert!(!output.contains("node_modules"));
        assert!(!output.contains(".git"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn grep_respects_max_matches() {
        let dir = std::env::temp_dir().join("openhuman_test_grep_max");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let mut text = String::new();
        for _ in 0..50 {
            text.push_str("hit\n");
        }
        tokio::fs::write(dir.join("many.txt"), text).await.unwrap();

        let tool = GrepTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"pattern": "hit", "max_matches": 5}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("truncated"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
