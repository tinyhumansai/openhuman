use super::traits::{Tool, ToolResult};
use crate::alphahuman::security::{AutonomyLevel, SecurityPolicy};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Git operations tool for structured repository management.
/// Provides safe, parsed git operations with JSON output.
pub struct GitOperationsTool {
    security: Arc<SecurityPolicy>,
    workspace_dir: std::path::PathBuf,
}

impl GitOperationsTool {
    pub fn new(security: Arc<SecurityPolicy>, workspace_dir: std::path::PathBuf) -> Self {
        Self {
            security,
            workspace_dir,
        }
    }

    /// Sanitize git arguments to prevent injection attacks
    fn sanitize_git_args(&self, args: &str) -> anyhow::Result<Vec<String>> {
        let mut result = Vec::new();
        for arg in args.split_whitespace() {
            // Block dangerous git options that could lead to command injection
            let arg_lower = arg.to_lowercase();
            if arg_lower.starts_with("--exec=")
                || arg_lower.starts_with("--upload-pack=")
                || arg_lower.starts_with("--receive-pack=")
                || arg_lower.starts_with("--pager=")
                || arg_lower.starts_with("--editor=")
                || arg_lower == "--no-verify"
                || arg_lower.contains("$(")
                || arg_lower.contains('`')
                || arg.contains('|')
                || arg.contains(';')
                || arg.contains('>')
            {
                anyhow::bail!("Blocked potentially dangerous git argument: {arg}");
            }
            // Block `-c` config injection (exact match or `-c=...` prefix).
            // This must not false-positive on `--cached` or `-cached`.
            if arg_lower == "-c" || arg_lower.starts_with("-c=") {
                anyhow::bail!("Blocked potentially dangerous git argument: {arg}");
            }
            result.push(arg.to_string());
        }
        Ok(result)
    }

    /// Check if an operation requires write access
    fn requires_write_access(&self, operation: &str) -> bool {
        matches!(
            operation,
            "commit" | "add" | "checkout" | "stash" | "reset" | "revert"
        )
    }

    /// Check if an operation is read-only
    fn is_read_only(&self, operation: &str) -> bool {
        matches!(
            operation,
            "status" | "diff" | "log" | "show" | "branch" | "rev-parse"
        )
    }

    async fn run_git_command(&self, args: &[&str]) -> anyhow::Result<String> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(&self.workspace_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git command failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn git_status(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = self
            .run_git_command(&["status", "--porcelain=2", "--branch"])
            .await?;

        // Parse git status output into structured format
        let mut result = serde_json::Map::new();
        let mut branch = String::new();
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in output.lines() {
            if line.starts_with("# branch.head ") {
                branch = line.trim_start_matches("# branch.head ").to_string();
            } else if let Some(rest) = line.strip_prefix("1 ") {
                // Ordinary changed entry
                let mut parts = rest.splitn(3, ' ');
                if let (Some(staging), Some(path)) = (parts.next(), parts.next()) {
                    if !staging.is_empty() {
                        let status_char = staging.chars().next().unwrap_or(' ');
                        if status_char != '.' && status_char != ' ' {
                            staged.push(json!({"path": path, "status": status_char}));
                        }
                        let status_char = staging.chars().nth(1).unwrap_or(' ');
                        if status_char != '.' && status_char != ' ' {
                            unstaged.push(json!({"path": path, "status": status_char}));
                        }
                    }
                }
            } else if let Some(rest) = line.strip_prefix("? ") {
                untracked.push(rest.to_string());
            }
        }

        result.insert("branch".to_string(), json!(branch));
        result.insert("staged".to_string(), json!(staged));
        result.insert("unstaged".to_string(), json!(unstaged));
        result.insert("untracked".to_string(), json!(untracked));
        result.insert(
            "clean".to_string(),
            json!(staged.is_empty() && unstaged.is_empty() && untracked.is_empty()),
        );

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result).unwrap_or_default(),
            error: None,
        })
    }

    async fn git_diff(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let files = args.get("files").and_then(|v| v.as_str()).unwrap_or(".");
        let cached = args
            .get("cached")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Validate files argument against injection patterns
        self.sanitize_git_args(files)?;

        let mut git_args = vec!["diff", "--unified=3"];
        if cached {
            git_args.push("--cached");
        }
        git_args.push("--");
        git_args.push(files);

        let output = self.run_git_command(&git_args).await?;

        // Parse diff into structured hunks
        let mut result = serde_json::Map::new();
        let mut hunks = Vec::new();
        let mut current_file = String::new();
        let mut current_hunk = serde_json::Map::new();
        let mut lines = Vec::new();

        for line in output.lines() {
            if line.starts_with("diff --git ") {
                if !lines.is_empty() {
                    current_hunk.insert("lines".to_string(), json!(lines));
                    if !current_hunk.is_empty() {
                        hunks.push(serde_json::Value::Object(current_hunk.clone()));
                    }
                    lines = Vec::new();
                    current_hunk = serde_json::Map::new();
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    current_file = parts[3].trim_start_matches("b/").to_string();
                    current_hunk.insert("file".to_string(), json!(current_file));
                }
            } else if line.starts_with("@@ ") {
                if !lines.is_empty() {
                    current_hunk.insert("lines".to_string(), json!(lines));
                    if !current_hunk.is_empty() {
                        hunks.push(serde_json::Value::Object(current_hunk.clone()));
                    }
                    lines = Vec::new();
                    current_hunk = serde_json::Map::new();
                    current_hunk.insert("file".to_string(), json!(current_file));
                }
                current_hunk.insert("header".to_string(), json!(line));
            } else if !line.is_empty() {
                lines.push(json!({
                    "text": line,
                    "type": if line.starts_with('+') { "add" }
                           else if line.starts_with('-') { "delete" }
                           else { "context" }
                }));
            }
        }

        if !lines.is_empty() {
            current_hunk.insert("lines".to_string(), json!(lines));
            if !current_hunk.is_empty() {
                hunks.push(serde_json::Value::Object(current_hunk));
            }
        }

        result.insert("hunks".to_string(), json!(hunks));
        result.insert("file_count".to_string(), json!(hunks.len()));

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result).unwrap_or_default(),
            error: None,
        })
    }

    async fn git_log(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let limit_raw = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
        let limit = usize::try_from(limit_raw).unwrap_or(usize::MAX).min(1000);
        let limit_str = limit.to_string();

        let output = self
            .run_git_command(&[
                "log",
                &format!("-{limit_str}"),
                "--pretty=format:%H|%an|%ae|%ad|%s",
                "--date=iso",
            ])
            .await?;

        let mut commits = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 5 {
                commits.push(json!({
                    "hash": parts[0],
                    "author": parts[1],
                    "email": parts[2],
                    "date": parts[3],
                    "message": parts[4]
                }));
            }
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({ "commits": commits }))
                .unwrap_or_default(),
            error: None,
        })
    }

    async fn git_branch(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = self
            .run_git_command(&["branch", "--format=%(refname:short)|%(HEAD)"])
            .await?;

        let mut branches = Vec::new();
        let mut current = String::new();

        for line in output.lines() {
            if let Some((name, head)) = line.split_once('|') {
                let is_current = head == "*";
                if is_current {
                    current = name.to_string();
                }
                branches.push(json!({
                    "name": name,
                    "current": is_current
                }));
            }
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "current": current,
                "branches": branches
            }))
            .unwrap_or_default(),
            error: None,
        })
    }

    fn truncate_commit_message(message: &str) -> String {
        if message.chars().count() > 2000 {
            format!("{}...", message.chars().take(1997).collect::<String>())
        } else {
            message.to_string()
        }
    }

    async fn git_commit(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter"))?;

        // Sanitize commit message
        let sanitized = message
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        if sanitized.is_empty() {
            anyhow::bail!("Commit message cannot be empty");
        }

        // Limit message length
        let message = Self::truncate_commit_message(&sanitized);

        let output = self.run_git_command(&["commit", "-m", &message]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Committed: {message}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Commit failed: {e}")),
            }),
        }
    }

    async fn git_add(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let paths = args
            .get("paths")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'paths' parameter"))?;

        // Validate paths against injection patterns
        self.sanitize_git_args(paths)?;

        let output = self.run_git_command(&["add", "--", paths]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Staged: {paths}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Add failed: {e}")),
            }),
        }
    }

    async fn git_checkout(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let branch = args
            .get("branch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'branch' parameter"))?;

        // Sanitize branch name
        let sanitized = self.sanitize_git_args(branch)?;

        if sanitized.is_empty() || sanitized.len() > 1 {
            anyhow::bail!("Invalid branch specification");
        }

        let branch_name = &sanitized[0];

        // Block dangerous branch names
        if branch_name.contains('@') || branch_name.contains('^') || branch_name.contains('~') {
            anyhow::bail!("Branch name contains invalid characters");
        }

        let output = self.run_git_command(&["checkout", branch_name]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Switched to branch: {branch_name}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Checkout failed: {e}")),
            }),
        }
    }

    async fn git_stash(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("push");

        let output = match action {
            "push" | "save" => {
                self.run_git_command(&["stash", "push", "-m", "auto-stash"])
                    .await
            }
            "pop" => self.run_git_command(&["stash", "pop"]).await,
            "list" => self.run_git_command(&["stash", "list"]).await,
            "drop" => {
                let index_raw = args.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let index = i32::try_from(index_raw)
                    .map_err(|_| anyhow::anyhow!("stash index too large: {index_raw}"))?;
                self.run_git_command(&["stash", "drop", &format!("stash@{{{index}}}")])
                    .await
            }
            _ => anyhow::bail!("Unknown stash action: {action}. Use: push, pop, list, drop"),
        };

        match output {
            Ok(out) => Ok(ToolResult {
                success: true,
                output: out,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Stash {action} failed: {e}")),
            }),
        }
    }
}

#[async_trait]
impl Tool for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }

    fn description(&self) -> &str {
        "Perform structured Git operations (status, diff, log, branch, commit, add, checkout, stash). Provides parsed JSON output and integrates with security policy for autonomy controls."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "branch", "commit", "add", "checkout", "stash"],
                    "description": "Git operation to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for 'commit' operation)"
                },
                "paths": {
                    "type": "string",
                    "description": "File paths to stage (for 'add' operation)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name (for 'checkout' operation)"
                },
                "files": {
                    "type": "string",
                    "description": "File or path to diff (for 'diff' operation, default: '.')"
                },
                "cached": {
                    "type": "boolean",
                    "description": "Show staged changes (for 'diff' operation)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of log entries (for 'log' operation, default: 10)"
                },
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop"],
                    "description": "Stash action (for 'stash' operation)"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index (for 'stash' with 'drop' action)"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let operation = match args.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'operation' parameter".into()),
                });
            }
        };

        // Check if we're in a git repository
        if !self.workspace_dir.join(".git").exists() {
            // Try to find .git in parent directories
            let mut current_dir = self.workspace_dir.as_path();
            let mut found_git = false;
            while current_dir.parent().is_some() {
                if current_dir.join(".git").exists() {
                    found_git = true;
                    break;
                }
                current_dir = current_dir.parent().unwrap();
            }

            if !found_git {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Not in a git repository".into()),
                });
            }
        }

        // Check autonomy level for write operations
        if self.requires_write_access(operation) {
            if !self.security.can_act() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "Action blocked: git write operations require higher autonomy level".into(),
                    ),
                });
            }

            match self.security.autonomy {
                AutonomyLevel::ReadOnly => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("Action blocked: read-only mode".into()),
                    });
                }
                AutonomyLevel::Supervised | AutonomyLevel::Full => {}
            }
        }

        // Record action for rate limiting
        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        // Execute the requested operation
        match operation {
            "status" => self.git_status(args).await,
            "diff" => self.git_diff(args).await,
            "log" => self.git_log(args).await,
            "branch" => self.git_branch(args).await,
            "commit" => self.git_commit(args).await,
            "add" => self.git_add(args).await,
            "checkout" => self.git_checkout(args).await,
            "stash" => self.git_stash(args).await,
            _ => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown operation: {operation}")),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphahuman::security::SecurityPolicy;
    use tempfile::TempDir;

    fn test_tool(dir: &std::path::Path) -> GitOperationsTool {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        });
        GitOperationsTool::new(security, dir.to_path_buf())
    }

    #[test]
    fn sanitize_git_blocks_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Should block dangerous arguments
        assert!(tool.sanitize_git_args("--exec=rm -rf /").is_err());
        assert!(tool.sanitize_git_args("$(echo pwned)").is_err());
        assert!(tool.sanitize_git_args("`malicious`").is_err());
        assert!(tool.sanitize_git_args("arg | cat").is_err());
        assert!(tool.sanitize_git_args("arg; rm file").is_err());
    }

    #[test]
    fn sanitize_git_blocks_pager_editor_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("--pager=less").is_err());
        assert!(tool.sanitize_git_args("--editor=vim").is_err());
    }

    #[test]
    fn sanitize_git_blocks_config_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Exact `-c` flag (config injection)
        assert!(tool.sanitize_git_args("-c core.sshCommand=evil").is_err());
        assert!(tool.sanitize_git_args("-c=core.pager=less").is_err());
    }

    #[test]
    fn sanitize_git_blocks_no_verify() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("--no-verify").is_err());
    }

    #[test]
    fn sanitize_git_blocks_redirect_in_args() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("file.txt > /tmp/out").is_err());
    }

    #[test]
    fn sanitize_git_cached_not_blocked() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // --cached must NOT be blocked by the `-c` check
        assert!(tool.sanitize_git_args("--cached").is_ok());
        // Other safe flags starting with -c prefix
        assert!(tool.sanitize_git_args("-cached").is_ok());
    }

    #[test]
    fn sanitize_git_allows_safe() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Should allow safe arguments
        assert!(tool.sanitize_git_args("main").is_ok());
        assert!(tool.sanitize_git_args("feature/test-branch").is_ok());
        assert!(tool.sanitize_git_args("--cached").is_ok());
        assert!(tool.sanitize_git_args("src/main.rs").is_ok());
        assert!(tool.sanitize_git_args(".").is_ok());
    }

    #[test]
    fn requires_write_detection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.requires_write_access("commit"));
        assert!(tool.requires_write_access("add"));
        assert!(tool.requires_write_access("checkout"));

        assert!(!tool.requires_write_access("status"));
        assert!(!tool.requires_write_access("diff"));
        assert!(!tool.requires_write_access("log"));
    }

    #[test]
    fn branch_is_not_write_gated() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Branch listing is read-only; it must not require write access
        assert!(!tool.requires_write_access("branch"));
        assert!(tool.is_read_only("branch"));
    }

    #[test]
    fn is_read_only_detection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.is_read_only("status"));
        assert!(tool.is_read_only("diff"));
        assert!(tool.is_read_only("log"));
        assert!(tool.is_read_only("branch"));

        assert!(!tool.is_read_only("commit"));
        assert!(!tool.is_read_only("add"));
    }

    #[tokio::test]
    async fn blocks_readonly_mode_for_write_ops() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        let result = tool
            .execute(json!({"operation": "commit", "message": "test"}))
            .await
            .unwrap();
        assert!(!result.success);
        // can_act() returns false for ReadOnly, so we get the "higher autonomy level" message
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("higher autonomy"));
    }

    #[tokio::test]
    async fn allows_branch_listing_in_readonly_mode() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository so the command can succeed
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        let result = tool.execute(json!({"operation": "branch"})).await.unwrap();
        // Branch listing must not be blocked by read-only autonomy
        let error_msg = result.error.as_deref().unwrap_or("");
        assert!(
            !error_msg.contains("read-only") && !error_msg.contains("higher autonomy"),
            "branch listing should not be blocked in read-only mode, got: {error_msg}"
        );
    }

    #[tokio::test]
    async fn allows_readonly_ops_in_readonly_mode() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        // This will fail because there's no git repo, but it shouldn't be blocked by autonomy
        let result = tool.execute(json!({"operation": "status"})).await.unwrap();
        // The error should be about git (not about autonomy/read-only mode)
        assert!(!result.success, "Expected failure due to missing git repo");
        let error_msg = result.error.as_deref().unwrap_or("");
        assert!(
            !error_msg.is_empty(),
            "Expected a git-related error message"
        );
        assert!(
            !error_msg.contains("read-only") && !error_msg.contains("autonomy"),
            "Error should be about git, not about autonomy restrictions: {error_msg}"
        );
    }

    #[tokio::test]
    async fn rejects_missing_operation() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Missing 'operation'"));
    }

    #[tokio::test]
    async fn rejects_unknown_operation() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let tool = test_tool(tmp.path());

        let result = tool.execute(json!({"operation": "push"})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Unknown operation"));
    }

    #[test]
    fn truncates_multibyte_commit_message_without_panicking() {
        let long = "🦀".repeat(2500);
        let truncated = GitOperationsTool::truncate_commit_message(&long);

        assert_eq!(truncated.chars().count(), 2000);
    }
}
