//! Thin query adapters for the module-owned CCR store and REPL algorithms.
//! OpenHuman authorizes tool-result artifacts; only their content crosses the bus.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyjuice_bus::repl::{ReplLimits, ReplOp};
use tinyjuice_bus::wire::{QueryError, QueryRequest, QueryResponse, QueryTarget};
use tinytools::{PermissionLevel, Tool, ToolResult};

/// The query tools TinyJuice declares. The footer suggests a subset of these.
pub const REPL_TOOL_NAMES: &[&str] = &["juice_find", "juice_extract", "juice_summarize"];

pub fn is_repl_tool(name: &str) -> bool {
    REPL_TOOL_NAMES.contains(&name)
}

/// Longest handle accepted. Real handles are 32 hex characters.
const MAX_HANDLE_LEN: usize = 64;

/// Largest persisted tool-result artifact read through the handle slot: the
/// same limit `file_read` applies, which is also the most the artifact store
/// ever writes.
const MAX_ARTIFACT_BYTES: u64 = tinytools_std::filesystem::FileReadTool::MAX_FILE_SIZE_BYTES;

/// The relative pointer form older stores handed out (`artifacts/tool-results/…`).
const RELATIVE_ARTIFACT_PREFIX: &str = "artifacts/tool-results/";

/// Contract requests are executed inside the compiled module.
#[async_trait]
pub(crate) trait QuerySource: Send + Sync {
    async fn query(&self, request: QueryRequest) -> Result<QueryResponse, String>;
}

struct ModuleSource {
    config: Option<Arc<crate::config::Config>>,
}

#[async_trait]
impl QuerySource for ModuleSource {
    async fn query(&self, request: QueryRequest) -> Result<QueryResponse, String> {
        match &self.config {
            Some(config) => super::query_for_config(config, request).await,
            None => super::query(request).await,
        }
    }
}

struct ModuleReplTool {
    name: String,
    op: String,
    description: String,
    schema: Value,
    cap: Option<usize>,
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
    /// `<workspace>/artifacts/tool-results`, when known: the one directory an
    /// `artifact_path` passed as the handle may be read from.
    artifacts_dir: Option<PathBuf>,
}

/// The three REPL tools, reading through the TinyJuice module. Without a
/// workspace they cannot resolve an `artifact_path`; see [`repl_tools_for`].
pub fn repl_tools() -> Vec<Box<dyn Tool>> {
    repl_tools_with(
        Arc::new(ModuleSource { config: None }),
        ReplLimits::default(),
    )
}

/// The REPL tools, or none while large results are not stored behind a handle
/// (compaction, router, CCR or `repl_handle_enabled` off): without a handle to
/// name there is nothing to query, and the schemas would be dead weight on
/// every turn.
pub fn repl_tools_for(config: &crate::config::Config) -> Vec<Box<dyn Tool>> {
    if !super::repl_handle_active(config) {
        return Vec::new();
    }
    log::debug!(
        "[tokenjuice][repl] registering {}",
        REPL_TOOL_NAMES.join(", ")
    );
    repl_tools_with_artifacts(
        Arc::new(ModuleSource {
            config: Some(Arc::new(config.clone())),
        }),
        ReplLimits::default(),
        Some(crate::security::policy::tool_result_artifacts_dir(
            &config.workspace_dir,
        )),
    )
}

/// Rebuild only the REPL tools a resumed transcript already declared. Their
/// schemas stay frozen while execution uses the current workspace and module
/// policy. This deliberately does not consult `repl_handle_active`: that flag
/// controls creating new handles, not reading handles a thread already has.
pub(crate) fn repl_tools_for_recorded(
    config: Option<&crate::config::Config>,
    workspace_dir: &Path,
    recorded: &[tinytools::ToolSpec],
) -> Vec<Box<dyn Tool>> {
    let declarations = tinyjuice_bus::tools::repl_tool_declarations();
    recorded
        .iter()
        .filter(|spec| is_repl_tool(&spec.name))
        .filter_map(|spec| {
            let declaration = declarations.iter().find(|item| item.name == spec.name)?;
            let limits = ReplLimits::default();
            Some(Box::new(ModuleReplTool {
                name: spec.name.clone(),
                op: declaration.op.clone(),
                description: spec.description.clone(),
                schema: spec.parameters.clone(),
                cap: Some(limits.max_output_chars.saturating_mul(2)),
                source: Arc::new(ModuleSource {
                    config: config.cloned().map(Arc::new),
                }),
                limits,
                artifacts_dir: Some(crate::security::policy::tool_result_artifacts_dir(
                    workspace_dir,
                )),
            }) as Box<dyn Tool>)
        })
        .collect()
}

pub(crate) fn repl_tools_with(
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
) -> Vec<Box<dyn Tool>> {
    repl_tools_with_artifacts(source, limits, None)
}

pub(crate) fn repl_tools_with_artifacts(
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
    artifacts_dir: Option<PathBuf>,
) -> Vec<Box<dyn Tool>> {
    tinyjuice_bus::tools::repl_tool_declarations()
        .into_iter()
        .map(|declaration| {
            Box::new(ModuleReplTool {
                name: declaration.name,
                op: declaration.op,
                description: declaration.description,
                schema: declaration.parameters,
                cap: Some(limits.max_output_chars.saturating_mul(2)),
                source: Arc::clone(&source),
                limits,
                artifacts_dir: artifacts_dir.clone(),
            }) as Box<dyn Tool>
        })
        .collect()
}

/// Accept a bare handle, or the `⟦tj:<handle>⟧` marker form.
fn normalize_handle(raw: &str) -> Option<&str> {
    let handle = raw
        .trim()
        .trim_start_matches("⟦tj:")
        .trim_end_matches('⟧')
        .trim();
    let valid = !handle.is_empty()
        && handle.len() <= MAX_HANDLE_LEN
        && handle.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some(handle)
}

/// What the model passed in the `handle` slot.
enum HandleArg {
    /// A CCR handle (bare or marker form), normalized.
    Handle(String),
    /// Something path-shaped: possibly a persisted tool-result artifact.
    Path(String),
    /// Neither: a tool call id, prose, an over-long token.
    Invalid,
}

fn classify_handle_arg(raw: &str) -> HandleArg {
    let trimmed = raw.trim();
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.ends_with(".txt") {
        return HandleArg::Path(trimmed.to_string());
    }
    match normalize_handle(trimmed) {
        Some(handle) => HandleArg::Handle(handle.to_string()),
        None => HandleArg::Invalid,
    }
}

fn not_a_handle_message() -> &'static str {
    "juice: that is not a juice handle. A handle is the 32-character hex token named in a \
     stored-output footer (or its `⟦tj:<handle>⟧` marker); a tool call id such as `call_…` \
     is not one. For a `[tool_result_preview]` envelope, pass its `artifact_path` as the \
     handle, or read that file with file_read."
}

fn artifact_refused(reason: &str) -> String {
    format!(
        "juice: {reason}. Only an `artifact_path` under the workspace's \
         artifacts/tool-results directory can stand in for a handle; read other files with \
         file_read."
    )
}

/// Read a persisted tool-result artifact named by `raw`, which must resolve
/// inside `dir` (the absolute pointer a detached store hands out, or the
/// legacy `artifacts/tool-results/…` relative form). Refuses `..`, a symlink
/// or anything else that resolves outside `dir`, a non-file, and a file larger
/// than `max_bytes`. Errors never quote the file's content.
pub(crate) fn read_tool_result_artifact(
    dir: &Path,
    raw: &str,
    max_bytes: u64,
) -> Result<String, String> {
    let raw = raw.trim();
    let path = Path::new(raw);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(artifact_refused("that path contains `..`"));
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let forward = raw.replace('\\', "/");
        match forward
            .trim_start_matches("./")
            .strip_prefix(RELATIVE_ARTIFACT_PREFIX)
        {
            Some(rest) if !rest.is_empty() => dir.join(rest),
            _ => return Err(artifact_refused("that path is not a tool-result artifact")),
        }
    };
    // The lexical gate accepts either spelling of `dir`: the store may hand
    // out the canonical path (macOS `/var` -> `/private/var`) while `dir` is
    // configured through a symlinked component. The canonical containment
    // check below is what actually enforces the boundary.
    let canonical_dir = std::fs::canonicalize(dir).ok();
    let lexically_inside = resolved.starts_with(dir)
        || canonical_dir
            .as_deref()
            .is_some_and(|canonical_dir| resolved.starts_with(canonical_dir));
    if !lexically_inside {
        return Err(artifact_refused("that path is not a tool-result artifact"));
    }
    let not_found = |_| artifact_refused("that artifact no longer exists");
    let canonical_dir = match canonical_dir {
        Some(canonical_dir) => canonical_dir,
        None => std::fs::canonicalize(dir).map_err(not_found)?,
    };
    let canonical = std::fs::canonicalize(&resolved).map_err(not_found)?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(artifact_refused(
            "that path resolves outside the tool-results directory",
        ));
    }
    let meta = std::fs::metadata(&canonical).map_err(not_found)?;
    if !meta.is_file() {
        return Err(artifact_refused("that path is not a file"));
    }
    if meta.len() > max_bytes {
        return Err(artifact_refused(&format!(
            "that artifact is {} bytes, over the {max_bytes}-byte limit",
            meta.len()
        )));
    }
    let bytes = std::fs::read(&canonical).map_err(not_found)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn miss_message() -> &'static str {
    "juice: that handle is no longer stored (evicted, or from an earlier session). \
     Do NOT re-run the same tool call to regenerate it: the result would be stored \
     again under a new handle. Work from the preview already shown, or re-run with \
     narrower arguments so the result is small enough to keep in full."
}

impl ModuleReplTool {
    async fn run_query(&self, target: QueryTarget, mut args: Value) -> anyhow::Result<ToolResult> {
        if let Some(object) = args.as_object_mut() {
            object.insert("op".into(), Value::String(self.op.clone()));
        }
        let op: ReplOp = match serde_json::from_value(args) {
            Ok(op) => op,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "juice: invalid arguments: {error}"
                )))
            }
        };
        let response = self
            .source
            .query(QueryRequest {
                target,
                op,
                limits: self.limits,
                context_token: None,
                scope: None,
            })
            .await;
        Ok(match response {
            Ok(Ok(output)) => ToolResult::json(serde_json::to_value(output)?),
            Ok(Err(QueryError::HandleNotFound)) => ToolResult::failed(miss_message()),
            Ok(Err(QueryError::InputTooLarge)) => {
                ToolResult::error("juice: input exceeds module limit")
            }
            Ok(Err(QueryError::InvalidPattern(pattern))) => {
                ToolResult::error(format!("juice: invalid pattern: {pattern}"))
            }
            Ok(Err(QueryError::EmptyQuery)) => ToolResult::error("juice: empty query"),
            Ok(Err(QueryError::Unsupported(operation))) => {
                ToolResult::error(format!("juice: {operation} support is not compiled in"))
            }
            Err(error) => ToolResult::error(format!("juice: {error}")),
        })
    }

    /// The handle slot held a path: query the persisted artifact it names.
    async fn execute_on_artifact(&self, path: String, args: Value) -> anyhow::Result<ToolResult> {
        let Some(dir) = self.artifacts_dir.clone() else {
            log::debug!(
                "[tokenjuice][repl] {} artifact path with no workspace",
                self.name
            );
            return Ok(ToolResult::error(not_a_handle_message()));
        };
        let read = crate::core::runtime::spawn_blocking_scoped(move || {
            read_tool_result_artifact(&dir, &path, MAX_ARTIFACT_BYTES)
        })
        .await
        .map_err(|e| anyhow::anyhow!("juice: artifact read task failed: {e}"))?;
        match read {
            Ok(content) => {
                log::debug!(
                    "[tokenjuice][repl] {} artifact bytes={}",
                    self.name,
                    content.len()
                );
                self.run_query(QueryTarget::Content { content }, args).await
            }
            Err(message) => {
                log::debug!("[tokenjuice][repl] {} artifact refused", self.name);
                Ok(ToolResult::error(message))
            }
        }
    }
}

#[async_trait]
impl Tool for ModuleReplTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let Some(raw) = args.get("handle").and_then(Value::as_str) else {
            return Ok(ToolResult::error("missing required argument: handle"));
        };
        let handle = match classify_handle_arg(raw) {
            HandleArg::Handle(handle) => handle,
            HandleArg::Path(path) => return self.execute_on_artifact(path, args).await,
            HandleArg::Invalid => {
                log::debug!("[tokenjuice][repl] {} rejected a non-handle", self.name);
                return Ok(ToolResult::error(not_a_handle_message()));
            }
        };
        self.run_query(QueryTarget::Handle { token: handle }, args)
            .await
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }

    fn external_effect(&self) -> bool {
        false
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        self.cap
    }
}

#[cfg(test)]
#[path = "repl_tools_tests.rs"]
mod tests;
