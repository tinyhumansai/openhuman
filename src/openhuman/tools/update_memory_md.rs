//! Tool: update_memory_md — append or update sections in MEMORY.md or SKILL.md.

use super::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Allowed workspace markdown files this tool may modify.
const ALLOWED_FILES: &[&str] = &["MEMORY.md", "SKILL.md"];

/// Appends or replaces a named section in MEMORY.md or SKILL.md.
///
/// Supports two actions:
/// - `append`: adds `content` to the end of the file.
/// - `replace_section`: locates the first `## {section_title}` heading and
///   replaces the body (lines until the next `##` heading or EOF) with `content`.
pub struct UpdateMemoryMdTool {
    workspace_dir: PathBuf,
}

impl UpdateMemoryMdTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for UpdateMemoryMdTool {
    fn name(&self) -> &str {
        "update_memory_md"
    }

    fn description(&self) -> &str {
        "Append or update sections in MEMORY.md or SKILL.md workspace files. \
         Use 'append' to add new notes at the end, or 'replace_section' to \
         overwrite the body under a named '## Section' heading."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["file", "action", "content"],
            "properties": {
                "file": {
                    "type": "string",
                    "enum": ["MEMORY.md", "SKILL.md"],
                    "description": "Which workspace markdown file to modify."
                },
                "action": {
                    "type": "string",
                    "enum": ["append", "replace_section"],
                    "description": "'append' adds content at the end; \
                                    'replace_section' replaces the body of the named section."
                },
                "section_title": {
                    "type": "string",
                    "description": "Required for 'replace_section': the heading text (without '## ')."
                },
                "content": {
                    "type": "string",
                    "description": "The markdown text to write."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let file = args
            .get("file")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'file' parameter"))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        // Guard: only allow MEMORY.md and SKILL.md.
        if !ALLOWED_FILES.contains(&file) {
            return Ok(ToolResult::error(format!(
                "File '{file}' is not allowed. Permitted files: MEMORY.md, SKILL.md"
            )));
        }

        let target_path = self.workspace_dir.join(file);

        // Prevent symlink-based workspace escape.
        let workspace_canon = self
            .workspace_dir
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Failed to canonicalize workspace: {e}"))?;
        // Check parent dir exists and canonicalize to detect symlinks.
        let parent = target_path.parent().unwrap_or(&self.workspace_dir);
        let parent_canon = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !parent_canon.starts_with(&workspace_canon) {
            return Ok(ToolResult::error(format!(
                "File path '{file}' resolves outside workspace"
            )));
        }

        tracing::debug!("[update_memory_md] action={action} file={file} path={target_path:?}");

        match action {
            "append" => self.do_append(&target_path, file, content).await,
            "replace_section" => {
                let section_title = args
                    .get("section_title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'section_title' is required for 'replace_section' action")
                    })?;
                self.do_replace_section(&target_path, file, section_title, content)
                    .await
            }
            other => Ok(ToolResult::error(format!(
                "Unknown action '{other}'. Use 'append' or 'replace_section'."
            ))),
        }
    }
}

impl UpdateMemoryMdTool {
    /// Append `content` to the end of `path`, creating the file if it does not exist.
    async fn do_append(
        &self,
        path: &std::path::Path,
        file: &str,
        content: &str,
    ) -> anyhow::Result<ToolResult> {
        // Read existing content (empty string if file not found).
        let existing = read_or_empty(path).await?;

        let separator = if existing.is_empty() || existing.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let new_content = format!("{existing}{separator}{content}\n");

        tokio::fs::write(path, &new_content)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write {file}: {e}"))?;

        let bytes = new_content.len();
        tracing::info!(
            "[update_memory_md] appended {} bytes to {file}",
            content.len()
        );

        Ok(ToolResult::success(format!(
            "Appended {} bytes to {file} ({bytes} bytes total).",
            content.len()
        )))
    }

    /// Replace the body of the section headed `## {section_title}` in `path`.
    ///
    /// If the section is not found it is appended as a new section at the end.
    async fn do_replace_section(
        &self,
        path: &std::path::Path,
        file: &str,
        section_title: &str,
        content: &str,
    ) -> anyhow::Result<ToolResult> {
        let existing = read_or_empty(path).await?;
        let heading = format!("## {section_title}");

        let lines: Vec<&str> = existing.lines().collect();
        let section_start = lines.iter().position(|l| l.trim() == heading.as_str());

        let new_file_content = if let Some(start_idx) = section_start {
            // Find where the next ## heading begins (or end of file).
            let body_start = start_idx + 1;
            let next_heading = lines[body_start..]
                .iter()
                .position(|l| l.starts_with("## "))
                .map(|rel| body_start + rel);

            let before: String = lines[..=start_idx].join("\n");
            let after: String = match next_heading {
                Some(end_idx) => {
                    let tail = lines[end_idx..].join("\n");
                    format!("\n{tail}")
                }
                None => String::new(),
            };

            // Ensure content is separated from the heading by a blank line.
            let body = if content.trim().is_empty() {
                String::new()
            } else {
                format!("\n{content}")
            };

            format!("{before}{body}{after}\n")
        } else {
            // Section not found — append it.
            tracing::debug!(
                "[update_memory_md] section '{section_title}' not found in {file}, appending"
            );
            let separator = if existing.is_empty() || existing.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            format!("{existing}{separator}{heading}\n{content}\n")
        };

        std::fs::write(path, &new_file_content)
            .map_err(|e| anyhow::anyhow!("Failed to write {file}: {e}"))?;

        tracing::info!(
            "[update_memory_md] replaced section '{}' in {file} ({} bytes written)",
            section_title,
            new_file_content.len()
        );

        Ok(ToolResult::success(format!(
            "Section '{}' updated in {file} ({} bytes).",
            section_title,
            new_file_content.len()
        )))
    }
}

/// Read file to string, returning an empty string when the file does not exist.
async fn read_or_empty(path: &std::path::Path) -> anyhow::Result<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(anyhow::anyhow!("Failed to read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(dir: &std::path::Path) -> UpdateMemoryMdTool {
        UpdateMemoryMdTool::new(dir.to_path_buf())
    }

    #[tokio::test]
    async fn append_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "action": "append",
                "content": "first note"
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{:?}", result.output());
        let text = std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
        assert!(text.contains("first note"));
    }

    #[tokio::test]
    async fn append_adds_to_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(&path, "existing\n").unwrap();
        let tool = make_tool(dir.path());
        tool.execute(json!({
            "file": "MEMORY.md",
            "action": "append",
            "content": "second note"
        }))
        .await
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("existing"));
        assert!(text.contains("second note"));
    }

    #[tokio::test]
    async fn replace_section_overwrites_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(&path, "## Lessons\nold body\n## Other\nkept\n").unwrap();
        let tool = make_tool(dir.path());
        tool.execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "section_title": "Lessons",
            "content": "new body"
        }))
        .await
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("new body"), "new body missing: {text}");
        assert!(
            !text.contains("old body"),
            "old body should be gone: {text}"
        );
        assert!(text.contains("## Other"), "other section missing: {text}");
        assert!(text.contains("kept"), "other section body missing: {text}");
    }

    #[tokio::test]
    async fn replace_section_appends_when_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, "# Header\n").unwrap();
        let tool = make_tool(dir.path());
        tool.execute(json!({
            "file": "SKILL.md",
            "action": "replace_section",
            "section_title": "New Section",
            "content": "brand new"
        }))
        .await
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("## New Section"), "heading missing: {text}");
        assert!(text.contains("brand new"), "content missing: {text}");
    }

    #[tokio::test]
    async fn rejects_disallowed_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "../../etc/passwd",
                "action": "append",
                "content": "evil"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not allowed"));
    }
}
