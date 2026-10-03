//! Agent tools `juice_find`, `juice_extract` and `juice_summarize`: inspect a
//! tool result the TinyJuice module stored behind a handle.
//!
//! With `[tokenjuice] repl_handle_enabled` on, the module replaces a large
//! result with a stats line, a short head and a handle instead of a summary
//! (see `docs/repl-tools.md` in the TinyJuice repository). These tools are how
//! the model queries what is behind that handle without reading it whole.
//!
//! TinyJuice owns the ops and the tool declarations
//! (`tinyjuice::repl::tools::repl_tools`, the `tinytools` feature). Its CCR
//! store, though, lives inside the module behind the bus, so a store handed to
//! `repl_tools` in this process would be empty. Each wrapper here therefore
//! fetches the original through the same `Retrieve` call `juice_retrieve`
//! makes, gives the stock tool a one-entry store holding it, and returns what
//! the stock tool answers. The ops, argument parsing, size caps and
//! read-only/concurrency flags are TinyJuice's, unchanged.
//!
//! Read-only, no side effects, no path or network access. Nothing here logs
//! the handle's content or the query.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyjuice::cache::store::{CcrPutResult, CcrStore};
use tinyjuice::repl::ReplLimits;
use tinytools::{PermissionLevel, Tool, ToolResult};

/// The query tools TinyJuice declares. The footer suggests a subset of these.
pub const REPL_TOOL_NAMES: &[&str] = &["juice_find", "juice_extract", "juice_summarize"];

pub fn is_repl_tool(name: &str) -> bool {
    REPL_TOOL_NAMES.contains(&name)
}

/// Longest handle accepted. Real handles are 32 hex characters.
const MAX_HANDLE_LEN: usize = 64;

/// Where the original behind a handle comes from.
#[async_trait]
pub(crate) trait OriginalSource: Send + Sync {
    /// `Ok(None)` is an unknown or evicted handle.
    async fn original(&self, handle: &str) -> Result<Option<String>, String>;
}

/// The TinyJuice module's CCR store, over the bus.
struct ModuleSource;

#[async_trait]
impl OriginalSource for ModuleSource {
    async fn original(&self, handle: &str) -> Result<Option<String>, String> {
        super::retrieve(handle.to_string(), None).await
    }
}

/// A [`CcrStore`] holding the one original a call is about.
struct OneEntryStore {
    token: String,
    content: String,
}

impl CcrStore for OneEntryStore {
    fn put(&self, _content: &str) -> CcrPutResult {
        // Read-only ops never store; report "not retained" rather than lie.
        CcrPutResult::new(String::new(), false)
    }

    fn get(&self, token: &str) -> Option<String> {
        (token == self.token).then(|| self.content.clone())
    }
}

struct ModuleReplTool {
    name: String,
    description: String,
    schema: Value,
    cap: Option<usize>,
    source: Arc<dyn OriginalSource>,
    limits: ReplLimits,
}

/// The three REPL tools, reading through the TinyJuice module.
pub fn repl_tools() -> Vec<Box<dyn Tool>> {
    repl_tools_with(Arc::new(ModuleSource), ReplLimits::default())
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
    repl_tools()
}

pub(crate) fn repl_tools_with(
    source: Arc<dyn OriginalSource>,
    limits: ReplLimits,
) -> Vec<Box<dyn Tool>> {
    // The declarations come from TinyJuice; the probe store is never read.
    let probe: Arc<dyn CcrStore> = Arc::new(OneEntryStore {
        token: String::new(),
        content: String::new(),
    });
    tinyjuice::repl::tools::repl_tools(probe, limits)
        .into_iter()
        .map(|inner| {
            Box::new(ModuleReplTool {
                name: inner.name().to_string(),
                description: inner.description().to_string(),
                schema: inner.parameters_schema(),
                cap: inner.max_result_size_chars(),
                source: Arc::clone(&source),
                limits,
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

fn miss_message() -> &'static str {
    "juice: that handle is no longer stored (evicted, or from an earlier session). \
     Do NOT re-run the same tool call to regenerate it: the result would be stored \
     again under a new handle. Work from the preview already shown, or re-run with \
     narrower arguments so the result is small enough to keep in full."
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

    async fn execute(&self, mut args: Value) -> anyhow::Result<ToolResult> {
        let Some(raw) = args.get("handle").and_then(Value::as_str) else {
            return Ok(ToolResult::error("missing required argument: handle"));
        };
        let Some(handle) = normalize_handle(raw).map(str::to_string) else {
            return Ok(ToolResult::error(
                "invalid handle: pass the handle from the stored-output footer",
            ));
        };
        let content = match self.source.original(&handle).await {
            Ok(Some(content)) => content,
            Ok(None) => {
                log::debug!("[tokenjuice][repl] {} handle miss", self.name);
                return Ok(ToolResult::failed(miss_message()));
            }
            Err(error) => {
                log::debug!("[tokenjuice][repl] {} source error: {error}", self.name);
                return Ok(ToolResult::error(format!("juice: {error}")));
            }
        };
        log::debug!(
            "[tokenjuice][repl] {} handle={handle} bytes={}",
            self.name,
            content.len()
        );
        if let Some(object) = args.as_object_mut() {
            object.insert("handle".into(), Value::String(handle.clone()));
        }
        let store: Arc<dyn CcrStore> = Arc::new(OneEntryStore {
            token: handle,
            content,
        });
        let Some(tool) = tinyjuice::repl::tools::repl_tools(store, self.limits)
            .into_iter()
            .find(|tool| tool.name() == self.name)
        else {
            return Ok(ToolResult::error("juice: unknown repl tool"));
        };
        tool.execute(args).await
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
