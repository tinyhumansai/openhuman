//! Tool: detect_tools — report which developer toolchains are installed on PATH.
//!
//! Lets the agent ground its plans in what the host actually has rather than
//! assuming. Read-only: it only scans `$PATH` for executables (no subprocesses,
//! no writes), so it is safe in every access mode.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Common developer tools probed when the caller doesn't specify a list.
const DEFAULT_CANDIDATES: &[&str] = &[
    "node", "npm", "npx", "pnpm", "yarn", "bun", "deno", "python3", "python", "pip3", "pip", "uv",
    "pipx", "cargo", "rustc", "go", "gcc", "cc", "clang", "make", "git", "gh", "docker", "podman",
    "kubectl", "rg", "jq", "fd", "curl", "wget",
];

pub struct DetectToolsTool;

impl DetectToolsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DetectToolsTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Locate `name` on `$PATH`, honoring `PATHEXT` on Windows. Returns the first
/// matching executable path, or `None` if not found.
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                // On Unix a plain `is_file()` can match a non-executable file and
                // falsely report the tool as available; require the exec bit.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let is_exec = std::fs::metadata(&candidate)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false);
                    if is_exec {
                        return Some(candidate);
                    }
                }
                #[cfg(not(unix))]
                {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[async_trait]
impl Tool for DetectToolsTool {
    fn name(&self) -> &str {
        "detect_tools"
    }

    fn description(&self) -> &str {
        "Detect which developer tools / language runtimes are installed on the host PATH \
         (e.g. node, python3, cargo, docker, git, rg). Use this before assuming a tool \
         exists or before proposing to install one. Read-only — scans PATH only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of tool names to probe. If omitted, a default \
                                    catalog of common developer tools is probed."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let requested: Vec<String> = args
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let candidates: Vec<String> = if requested.is_empty() {
            DEFAULT_CANDIDATES
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        } else {
            requested
        };

        let mut available = Vec::new();
        let mut missing = Vec::new();
        for name in &candidates {
            match find_on_path(name) {
                Some(p) => available.push(json!({
                    "name": name,
                    "path": p.to_string_lossy(),
                })),
                None => missing.push(name.clone()),
            }
        }

        tracing::debug!(
            probed = candidates.len(),
            available = available.len(),
            "[detect_tools] PATH scan complete"
        );
        let payload = json!({
            "available": available,
            "missing": missing,
            "probed": candidates.len(),
        });
        Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let tool = DetectToolsTool::new();
        assert_eq!(tool.name(), "detect_tools");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[tokio::test]
    async fn missing_tool_reported_missing() {
        let tool = DetectToolsTool::new();
        let result = tool
            .execute(json!({ "tools": ["definitely_not_a_real_binary_xyz_123"] }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert_eq!(payload["probed"], 1);
        assert_eq!(payload["available"].as_array().unwrap().len(), 0);
        assert_eq!(
            payload["missing"].as_array().unwrap()[0],
            "definitely_not_a_real_binary_xyz_123"
        );
    }

    #[tokio::test]
    async fn available_plus_missing_equals_probed() {
        let tool = DetectToolsTool::new();
        let result = tool
            .execute(json!({ "tools": ["sh", "definitely_not_a_real_binary_xyz_123"] }))
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        let avail = payload["available"].as_array().unwrap().len();
        let miss = payload["missing"].as_array().unwrap().len();
        assert_eq!(avail + miss, 2);
    }
}
