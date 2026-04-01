//! Self-healing interceptor — auto-polyfill when commands are missing.
//!
//! When the Code Executor's shell tool returns "command not found" or similar,
//! the interceptor spawns a ToolMaker sub-agent to write a polyfill script,
//! then retries the original command.

use crate::openhuman::tools::ToolResult;
use std::path::{Path, PathBuf};

/// Maximum number of self-heal attempts per unique command.
const MAX_HEAL_ATTEMPTS: u8 = 2;

/// Patterns in tool error output that indicate a missing command/binary.
const MISSING_CMD_PATTERNS: &[&str] = &[
    "command not found",
    "not found",
    "not installed",
    "No such file or directory",
    "not recognized as an internal or external command",
    "is not recognized",
    "unable to find",
];

/// Interceptor that detects missing-command errors and spawns ToolMaker agents.
pub struct SelfHealingInterceptor {
    /// Directory where polyfill scripts are written.
    polyfill_dir: PathBuf,
    /// Whether self-healing is enabled.
    enabled: bool,
    /// Track heal attempts per command to enforce MAX_HEAL_ATTEMPTS.
    attempts: std::collections::HashMap<String, u8>,
}

impl SelfHealingInterceptor {
    pub fn new(workspace_dir: &Path, enabled: bool) -> Self {
        let polyfill_dir = workspace_dir.join("polyfills");
        Self {
            polyfill_dir,
            enabled,
            attempts: std::collections::HashMap::new(),
        }
    }

    /// Check if a tool result indicates a missing command that can be self-healed.
    ///
    /// Returns `Some(command_name)` if the error matches a known missing-command pattern
    /// and we haven't exceeded the retry limit.
    pub fn detect_missing_command(&mut self, result: &ToolResult) -> Option<String> {
        if !self.enabled || result.success {
            return None;
        }

        let error_text = result.error.as_deref().unwrap_or("").to_lowercase();
        let output_text = result.output.to_lowercase();
        let combined = format!("{error_text} {output_text}");

        // Check if the error matches any missing-command pattern.
        let is_missing = MISSING_CMD_PATTERNS
            .iter()
            .any(|pattern| combined.contains(&pattern.to_lowercase()));

        if !is_missing {
            return None;
        }

        // Try to extract the command name from the error.
        let cmd = extract_command_name(&combined)?;

        // Check retry limit.
        let count = self.attempts.entry(cmd.clone()).or_insert(0);
        if *count >= MAX_HEAL_ATTEMPTS {
            tracing::debug!(
                "[self-healing] max attempts ({MAX_HEAL_ATTEMPTS}) reached for command: {cmd}"
            );
            return None;
        }
        *count += 1;

        tracing::info!(
            "[self-healing] detected missing command: {cmd} (attempt {}/{})",
            *count,
            MAX_HEAL_ATTEMPTS
        );

        Some(cmd)
    }

    /// Build the prompt for the ToolMaker sub-agent.
    pub fn tool_maker_prompt(&self, missing_command: &str, original_context: &str) -> String {
        format!(
            "The command `{missing_command}` is not available in this environment.\n\
             \n\
             Write a polyfill script that accomplishes the equivalent functionality.\n\
             Save it to: {polyfill_dir}/{missing_command}\n\
             Make it executable with `chmod +x`.\n\
             \n\
             Original context:\n{original_context}\n\
             \n\
             Requirements:\n\
             - Use only standard tools likely available (bash, python3, awk, sed, curl).\n\
             - The script should accept the same arguments as the original command.\n\
             - Keep it minimal — just enough to accomplish the immediate task.\n\
             - Do NOT install packages or use sudo.",
            polyfill_dir = self.polyfill_dir.display()
        )
    }

    /// Get the polyfill directory path.
    pub fn polyfill_dir(&self) -> &Path {
        &self.polyfill_dir
    }

    /// Ensure the polyfill directory exists.
    pub async fn ensure_polyfill_dir(&self) -> anyhow::Result<()> {
        if !self.polyfill_dir.exists() {
            tokio::fs::create_dir_all(&self.polyfill_dir).await?;
            tracing::debug!(
                "[self-healing] created polyfill directory: {}",
                self.polyfill_dir.display()
            );
        }
        Ok(())
    }

    /// Reset attempt counters (e.g. between sessions).
    pub fn reset(&mut self) {
        self.attempts.clear();
    }
}

/// Try to extract a command name from an error message.
///
/// Handles patterns like:
/// - "bash: foo: command not found"
/// - "sh: 1: foo: not found"
/// - "'foo' is not recognized"
fn extract_command_name(error: &str) -> Option<String> {
    // Pattern: "bash: CMD: command not found"
    if let Some(idx) = error.find(": command not found") {
        let before = &error[..idx];
        if let Some(colon_idx) = before.rfind(": ") {
            let cmd = before[colon_idx + 2..].trim();
            if !cmd.is_empty() && cmd.len() < 64 {
                return Some(cmd.to_string());
            }
        }
        // Try without preceding colon.
        let cmd = before.trim();
        if let Some(last_word) = cmd.split_whitespace().last() {
            if last_word.len() < 64 {
                return Some(last_word.to_string());
            }
        }
    }

    // Pattern: "sh: N: CMD: not found"
    if error.contains(": not found") {
        let parts: Vec<&str> = error.split(':').collect();
        if parts.len() >= 3 {
            let candidate = parts[parts.len() - 2].trim();
            if !candidate.is_empty()
                && candidate.len() < 64
                && !candidate.chars().all(|c| c.is_ascii_digit())
            {
                return Some(candidate.to_string());
            }
        }
    }

    // Pattern: "'CMD' is not recognized"
    if error.contains("is not recognized") {
        let stripped = error.replace('\'', "").replace('"', "");
        if let Some(cmd) = stripped.split_whitespace().next() {
            if cmd.len() < 64 {
                return Some(cmd.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_error_result(error: &str) -> ToolResult {
        ToolResult {
            success: false,
            output: String::new(),
            error: Some(error.to_string()),
        }
    }

    #[test]
    fn detects_bash_command_not_found() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), true);
        let result = make_error_result("bash: jq: command not found");
        let cmd = interceptor.detect_missing_command(&result);
        assert_eq!(cmd, Some("jq".to_string()));
    }

    #[test]
    fn detects_sh_not_found() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), true);
        let result = make_error_result("sh: 1: nmap: not found");
        let cmd = interceptor.detect_missing_command(&result);
        assert_eq!(cmd, Some("nmap".to_string()));
    }

    #[test]
    fn respects_max_attempts() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), true);
        let result = make_error_result("bash: jq: command not found");

        // First two attempts should succeed.
        assert!(interceptor.detect_missing_command(&result).is_some());
        assert!(interceptor.detect_missing_command(&result).is_some());
        // Third should be None (max attempts reached).
        assert!(interceptor.detect_missing_command(&result).is_none());
    }

    #[test]
    fn ignores_successful_results() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), true);
        let result = ToolResult {
            success: true,
            output: "command not found".into(), // misleading output
            error: None,
        };
        assert!(interceptor.detect_missing_command(&result).is_none());
    }

    #[test]
    fn disabled_returns_none() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), false);
        let result = make_error_result("bash: jq: command not found");
        assert!(interceptor.detect_missing_command(&result).is_none());
    }

    #[test]
    fn reset_clears_attempts() {
        let mut interceptor = SelfHealingInterceptor::new(Path::new("/tmp"), true);
        let result = make_error_result("bash: jq: command not found");
        interceptor.detect_missing_command(&result);
        interceptor.detect_missing_command(&result);
        interceptor.reset();
        // After reset, should detect again.
        assert!(interceptor.detect_missing_command(&result).is_some());
    }

    #[test]
    fn tool_maker_prompt_includes_command() {
        let interceptor = SelfHealingInterceptor::new(Path::new("/workspace"), true);
        let prompt = interceptor.tool_maker_prompt("jq", "parse json output");
        assert!(prompt.contains("jq"));
        assert!(prompt.contains("/workspace/polyfills/jq"));
        assert!(prompt.contains("parse json output"));
    }
}
