//! `lsp` — capability-gated LSP query stub.
//!
//! Coding-harness baseline tool (issue #1205). The full LSP integration
//! (spawning language servers, JSON-RPC bridge, completion / hover /
//! definition / references) is large enough to live in its own
//! follow-up. This tool exists today as the **agent-facing surface +
//! capability gate** so:
//!
//! 1. The schema is stable: prompts and downstream callers can be
//!    written against `{ language, kind, file, line, character, symbol }`
//!    without churn when the real backend lands.
//! 2. The gate is observable: with `OPENHUMAN_LSP_ENABLED=1` set the
//!    tool registers; without it, it does not — so agents don't see a
//!    method that will always fail.
//! 3. When enabled but no backend is wired, the tool returns a clear
//!    "not yet implemented" error instead of silently misbehaving.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Env var that gates LSP tool registration.
pub const LSP_ENABLED_ENV: &str = "OPENHUMAN_LSP_ENABLED";

/// Returns true when the LSP capability gate is on. Accepts `1`, `true`,
/// `yes` (case-insensitive). Anything else (including unset) is off.
pub fn lsp_capability_enabled() -> bool {
    match std::env::var(LSP_ENABLED_ENV) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

pub struct LspTool;

impl LspTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LspTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Query a Language Server for code intelligence (definition, references, \
         hover, completion). Capability-gated: only registered when \
         OPENHUMAN_LSP_ENABLED=1. The server-spawning backend is a follow-up \
         — calls today return a `not yet implemented` error so callers can \
         feature-detect."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["definition", "references", "hover", "completion"],
                    "description": "Which LSP query to run."
                },
                "language": {
                    "type": "string",
                    "description": "Language id (e.g. `rust`, `typescript`, `python`)."
                },
                "file": { "type": "string", "description": "Workspace-relative file path." },
                "line": { "type": "integer", "minimum": 0 },
                "character": { "type": "integer", "minimum": 0 },
                "symbol": {
                    "type": "string",
                    "description": "Optional symbol name (used by some kinds, e.g. references)."
                }
            },
            "required": ["kind", "language", "file"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::error(
            "lsp backend not yet implemented — capability gate is on but no language server is wired. Track this in the follow-up issue."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Serialize env-var mutation across tests in this module so they
    /// don't race each other under Rust's default parallel runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn lsp_name_and_schema() {
        let tool = LspTool::new();
        assert_eq!(tool.name(), "lsp");
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("kind")));
        assert!(required.contains(&json!("language")));
        assert!(required.contains(&json!("file")));
    }

    #[tokio::test]
    async fn lsp_returns_not_implemented_error() {
        let tool = LspTool::new();
        let result = tool
            .execute(json!({
                "kind": "definition", "language": "rust", "file": "src/main.rs",
                "line": 0, "character": 0
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not yet implemented"));
    }

    #[test]
    fn lsp_capability_gate_off_by_default() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(LSP_ENABLED_ENV).ok();
        std::env::remove_var(LSP_ENABLED_ENV);
        assert!(!lsp_capability_enabled());
        if let Some(v) = prev {
            std::env::set_var(LSP_ENABLED_ENV, v);
        }
    }

    #[test]
    fn lsp_capability_gate_accepts_truthy_values() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(LSP_ENABLED_ENV).ok();
        for v in ["1", "true", "TRUE", "yes", "on"] {
            std::env::set_var(LSP_ENABLED_ENV, v);
            assert!(lsp_capability_enabled(), "expected truthy for {v:?}");
        }
        for v in ["0", "false", "no", "off", ""] {
            std::env::set_var(LSP_ENABLED_ENV, v);
            assert!(!lsp_capability_enabled(), "expected falsy for {v:?}");
        }
        match prev {
            Some(v) => std::env::set_var(LSP_ENABLED_ENV, v),
            None => std::env::remove_var(LSP_ENABLED_ENV),
        }
    }
}
