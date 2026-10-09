//! `show_ui`: lets a skill render its own HTML in the chat, in the same
//! sandboxed frame MCP widgets use.
//!
//! The document comes inline or from an `.html` file confined to the action
//! directory, the workspace, or the user's skill bundles. The frame it runs in
//! cannot call tools; it can only ask the host to open a link or prefill the
//! composer.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};

use super::cache::{self, InlineEntry};
use super::resolve::{MAX_RESOURCE_BYTES, MAX_STRUCTURED_BYTES};
use super::types::{McpUiPresentation, UiCsp, UiFlavor, UiResource, MCP_APP_MIME, MCP_UI_KIND};
use tinymcp::ui::extract_links;

/// The tool's registered name.
pub const SHOW_UI_TOOL: &str = "show_ui";

const MAX_TITLE_CHARS: usize = 80;

/// The `show_ui` tool.
pub struct ShowUiTool {
    roots: Vec<PathBuf>,
}

impl ShowUiTool {
    /// A tool reading files under `roots` only.
    #[must_use]
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// The tool for `config`: the action directory, the workspace and the
    /// user's skill bundles.
    #[must_use]
    pub fn for_config(config: &crate::config::Config) -> Self {
        let mut roots = vec![config.action_dir.clone(), config.workspace_dir.clone()];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".openhuman").join("skills"));
        }
        Self::new(roots)
    }
}

/// `path` resolved inside one of `roots`, refusing traversal, symlink escapes
/// and anything but `.html` / `.htm`.
///
/// # Errors
///
/// When the path is malformed, outside every root, or not an HTML file.
pub fn confine_path(roots: &[PathBuf], path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(path.trim());
    if path.trim().is_empty() || path.contains('\0') {
        return Err("path must not be empty".to_string());
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("path must not contain `..`".to_string());
    }
    let extension = requested
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("html" | "htm")) {
        return Err("only .html files can be shown".to_string());
    }
    for root in roots {
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            canonical_root.join(requested)
        };
        let Ok(resolved) = candidate.canonicalize() else {
            continue;
        };
        if resolved.starts_with(&canonical_root) && resolved.is_file() {
            return Ok(resolved);
        }
    }
    Err("the file is not inside the workspace or a skill bundle".to_string())
}

fn clean_title(raw: Option<&str>) -> Option<String> {
    let title: String = raw?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect();
    let title = title.trim().to_string();
    (!title.is_empty()).then_some(title)
}

#[async_trait]
impl Tool for ShowUiTool {
    fn name(&self) -> &str {
        SHOW_UI_TOOL
    }

    fn description(&self) -> &str {
        "Show an interactive HTML view inline in the chat, in a sandbox. Pass `html`, or `path` to an .html file in the workspace or a skill bundle, plus an optional `title` and a `data` object the page receives as its tool result. The page cannot call tools or reach the network; use it for visual output a skill produces."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "html": {"type": "string", "description": "A complete HTML document."},
                "path": {"type": "string", "description": "An .html file in the workspace or a skill bundle."},
                "title": {"type": "string", "description": "A short label for the view."},
                "data": {"type": "object", "description": "Data handed to the page."}
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let html_arg = args.get("html").and_then(Value::as_str);
        let path_arg = args.get("path").and_then(Value::as_str);
        let html = match (html_arg, path_arg) {
            (Some(html), None) => html.to_string(),
            (None, Some(path)) => {
                let resolved = match confine_path(&self.roots, path) {
                    Ok(resolved) => resolved,
                    Err(error) => return Ok(ToolResult::error(format!("show_ui: {error}"))),
                };
                let size = tokio::fs::metadata(&resolved).await?.len();
                if size > MAX_RESOURCE_BYTES as u64 {
                    return Ok(ToolResult::error("show_ui: the file is too large to show"));
                }
                tokio::fs::read_to_string(&resolved).await?
            }
            _ => {
                return Ok(ToolResult::error(
                    "show_ui: pass exactly one of `html` or `path`",
                ))
            }
        };
        if html.trim().is_empty() {
            return Ok(ToolResult::error("show_ui: the document is empty"));
        }
        if html.len() > MAX_RESOURCE_BYTES {
            return Ok(ToolResult::error(
                "show_ui: the document is too large to show",
            ));
        }
        let data = args
            .get("data")
            .filter(|value| value.is_object())
            .filter(|value| {
                serde_json::to_vec(value)
                    .map(|bytes| bytes.len() <= MAX_STRUCTURED_BYTES)
                    .unwrap_or(false)
            })
            .cloned();
        let title = clean_title(args.get("title").and_then(Value::as_str));
        let inline_id = cache::put_inline(InlineEntry {
            server_id: None,
            resource: UiResource {
                html,
                mime_type: MCP_APP_MIME.to_string(),
                csp: UiCsp::default(),
                permissions: None,
                prefers_border: true,
            },
        });
        let presentation = McpUiPresentation {
            kind: MCP_UI_KIND.to_string(),
            flavor: UiFlavor::HostInline,
            server_id: None,
            tool: SHOW_UI_TOOL.to_string(),
            resource_uri: None,
            inline_id: Some(inline_id),
            title: title.clone(),
            tool_input: data.clone().unwrap_or(Value::Null),
            structured_content: data.clone(),
            result_meta: None,
            links: extract_links(data.as_ref(), ""),
        };
        tracing::debug!(title = ?title, "[mcp_ui] show_ui rendered a host view");
        let mut result = ToolResult::success(match &title {
            Some(title) => format!("Shown \"{title}\" inline in the chat."),
            None => "Shown the view inline in the chat.".to_string(),
        });
        result.metadata = Some(presentation.to_metadata());
        Ok(result)
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
