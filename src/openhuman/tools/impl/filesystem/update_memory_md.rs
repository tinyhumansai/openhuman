//! Tool: update_memory_md — append or update sections in MEMORY.md or SKILL.md.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tinyagents::harness::tool::ToolExecutionContext;

/// Allowed workspace markdown files this tool may modify.
const ALLOWED_FILES: &[&str] = &["MEMORY.md", "SKILL.md"];

/// Process-global registry of per-workspace write locks (#4458).
///
/// `update_memory_md` performs a read-modify-write on `MEMORY.md`/`SKILL.md`.
/// The per-run `MemoryProtocolTracker` has zero cross-run awareness, so two
/// concurrent runs (parallel forks, cron) racing the same workspace file would
/// otherwise clobber each other's append. We serialize every write to a given
/// workspace directory through a shared async mutex, keyed by the canonicalized
/// (falling back to raw) workspace path, so concurrent index updates queue
/// instead of racing. Combined with the temp-file + atomic-rename write below,
/// a killed process can never leave a truncated file.
static WORKSPACE_WRITE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Monotonic counter for temp-file uniqueness within a process.
static TEMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Return (creating if needed) the shared async write lock for `workspace_dir`.
///
/// The lock key is the canonicalized workspace path when it resolves (so two
/// spellings of the same directory share one lock), else the raw path.
fn workspace_write_lock(workspace_dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let key = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let mut map = WORKSPACE_WRITE_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        map.entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Acquire an **inter-process** advisory write lock for `workspace_dir` (#4458).
///
/// [`WORKSPACE_WRITE_LOCKS`] only serializes writers inside this OS process, but
/// cron launches work via separate `tokio::process::Command` subprocesses that
/// don't share that mutex — so two cron runs could still clobber the same
/// `MEMORY.md` mid read-modify-write. This takes an `fs2` exclusive `flock` on a
/// sentinel `.memory-write.lock` file in the workspace; the returned `File`
/// holds the lock until it is dropped (end of the write). `flock` acquisition
/// blocks, so it runs on a blocking thread.
async fn acquire_cross_process_write_lock(workspace_dir: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = workspace_dir.join(".memory-write.lock");
    tokio::task::spawn_blocking(move || {
        use fs2::FileExt;
        if let Some(parent) = lock_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Reject a symlinked lock file: a project-controlled `.memory-write.lock`
        // symlink (including a dangling one) could otherwise redirect this
        // create/open/lock to a path OUTSIDE the already-containment-checked
        // workspace, bypassing the symlink hardening applied to MEMORY.md /
        // SKILL.md. If it exists it must be a regular file.
        if let Ok(meta) = std::fs::symlink_metadata(&lock_path) {
            if meta.file_type().is_symlink() {
                return Err(anyhow::anyhow!(
                    "workspace lock file {lock_path:?} is a symlink; refusing to follow it"
                ));
            }
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            // O_NOFOLLOW closes the TOCTOU window: if a symlink is swapped in
            // after the check above, the open fails (ELOOP) rather than follows.
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc::O_NOFOLLOW);
        }
        let file = opts
            .open(&lock_path)
            .map_err(|e| anyhow::anyhow!("open workspace lock file {lock_path:?}: {e}"))?;
        file.lock_exclusive()
            .map_err(|e| anyhow::anyhow!("acquire workspace write flock: {e}"))?;
        Ok::<std::fs::File, anyhow::Error>(file)
    })
    .await
    .map_err(|e| anyhow::anyhow!("workspace lock task join failed: {e}"))?
}

/// Atomically replace `path`'s contents with `content`.
///
/// Writes to a sibling temp file in the same directory (so the rename stays on
/// one filesystem and is atomic) and `rename`s it over the target. A crash
/// mid-write leaves either the old file or the complete new file — never a
/// half-written truncation. Callers MUST hold the per-workspace write lock so
/// the read-modify-write is serialized end-to-end.
async fn atomic_write(path: &Path, file: &str, content: &str) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target path has no parent directory"))?;
    let seq = TEMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".{file}.{}.{seq}.tmp", std::process::id());
    let tmp_path = dir.join(tmp_name);

    tracing::debug!(
        tmp = %tmp_path.display(),
        target = %path.display(),
        bytes = content.len(),
        "[update_memory_md] atomic write: staging temp file"
    );

    if let Err(e) = tokio::fs::write(&tmp_path, content).await {
        // Clean up a partially-written temp file so a failed stage doesn't
        // litter the workspace (CodeRabbit: temp not cleaned on initial write).
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(anyhow::anyhow!("Failed to stage temp file for {file}: {e}"));
    }

    if let Err(e) = tokio::fs::rename(&tmp_path, path).await {
        // Best-effort cleanup so a failed rename doesn't litter the workspace.
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(anyhow::anyhow!("Failed to atomically write {file}: {e}"));
    }

    tracing::debug!(
        target = %path.display(),
        "[update_memory_md] atomic write: rename committed"
    );
    Ok(())
}

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

    fn workspace_dir_for_context(&self, context: Option<&ToolExecutionContext>) -> PathBuf {
        if let Some(workspace) = context.and_then(|ctx| ctx.workspace.as_ref()) {
            tracing::debug!(
                workspace_root = %workspace.root.display(),
                policy_id = %workspace.policy_id,
                "[update_memory_md] using TinyAgents workspace descriptor as workspace dir"
            );
            return workspace.root.clone();
        }
        self.workspace_dir.clone()
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
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let workspace_dir = self.workspace_dir_for_context(context);
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

        let target_path = workspace_dir.join(file);

        // Prevent symlink-based workspace escape.
        let workspace_canon = self
            .workspace_dir_for_context(context)
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Failed to canonicalize workspace: {e}"))?;
        // Check parent dir exists and canonicalize to detect symlinks.
        let parent = target_path.parent().unwrap_or(&workspace_dir);
        let parent_canon = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !parent_canon.starts_with(&workspace_canon) {
            return Ok(ToolResult::error(format!(
                "File path '{file}' resolves outside workspace"
            )));
        }

        tracing::debug!("[update_memory_md] action={action} file={file} path={target_path:?}");

        // #4458: serialize the whole read-modify-write against concurrent runs
        // targeting the same workspace. The guard is held across read + atomic
        // write so no interleaving append can be lost.
        let lock = workspace_write_lock(&workspace_dir);
        let _guard = lock.lock().await;
        // Also take a cross-process advisory lock so cron subprocesses (which
        // don't share the in-process mutex above) can't clobber the same file
        // mid-RMW. Held across read + atomic write; released on drop.
        let _file_lock = acquire_cross_process_write_lock(&workspace_dir).await?;
        tracing::debug!(
            workspace = %workspace_dir.display(),
            "[update_memory_md] acquired per-workspace write lock (in-process + cross-process flock)"
        );

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

        atomic_write(path, file, &new_content).await?;

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

        atomic_write(path, file, &new_file_content).await?;

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
    async fn replace_section_with_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(&path, "## Notes\nold stuff\n## End\ndone\n").unwrap();
        let tool = make_tool(dir.path());
        tool.execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "section_title": "Notes",
            "content": ""
        }))
        .await
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("old stuff"),
            "old body should be gone: {text}"
        );
        assert!(text.contains("## End"), "other section missing: {text}");
    }

    #[tokio::test]
    async fn append_to_empty_memory_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(&path, "").unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "action": "append",
                "content": "first line"
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "unexpected error: {}", result.output());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("first line"));
    }

    #[tokio::test]
    async fn replace_section_creates_memory_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "action": "replace_section",
                "section_title": "First",
                "content": "hello"
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "unexpected error: {}", result.output());
        let text = std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
        assert!(text.contains("## First"));
        assert!(text.contains("hello"));
    }

    #[tokio::test]
    async fn rejects_unknown_action() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "action": "delete_all",
                "content": "x"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn replace_section_missing_section_title_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "action": "replace_section",
                "content": "x"
            }))
            .await;
        // May return Err or Ok with is_error
        match result {
            Ok(r) => assert!(r.is_error),
            Err(_) => {} // also acceptable
        }
    }

    #[test]
    fn tool_name_and_description() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        assert_eq!(tool.name(), "update_memory_md");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn parameters_schema_has_required_fields() {
        let dir = tempfile::tempdir().unwrap();
        let tool = make_tool(dir.path());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("file")));
        assert!(required.contains(&json!("action")));
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
