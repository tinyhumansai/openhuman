//! Authoritative tool-exposure middleware for model requests and execution.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{
    ContextualToolSelectionMiddleware, Middleware, ToolAllowlistMiddleware,
};
use tinyinference::model::ModelRequest;
use tinyinference::tool::ToolCall as TaToolCall;

/// Applies OpenHuman's already-authorized turn set through TinyAgents' native
/// contextual selector before every model call and its allowlist before every
/// tool call. Registration uses the same set, so hidden tools are neither
/// advertised nor executable.
pub(crate) struct OpenHumanToolExposureMiddleware {
    allowlist: ToolAllowlistMiddleware,
    selection: ContextualToolSelectionMiddleware,
    registered: std::collections::HashSet<String>,
    tags: Vec<String>,
    candidate_count: usize,
    reported: AtomicBool,
}

impl OpenHumanToolExposureMiddleware {
    /// Build the live layer from the same inputs used for tool registration.
    /// Allowlist semantics are **fail-closed** (issue #4452): `None` means "no
    /// filter supplied → all candidates visible"; `Some(set)` means "exactly the
    /// named tools", so `Some(empty)` is a genuine deny-all. This mirrors the
    /// registration loop in `assemble_turn_harness`, keeping model exposure and
    /// execution in step with what OpenHuman registered as callable.
    pub(crate) fn new(
        candidate_names: &[String],
        allowed: Option<&std::collections::HashSet<String>>,
        tags: Vec<String>,
    ) -> Self {
        // Effective visible set: `None` → every candidate; `Some(set)` → exactly
        // the candidates named in `set` (empty set → none). Fail-closed: a
        // candidate absent from a supplied `allowed` is excluded (not exposed).
        let registered: std::collections::HashSet<String> = match allowed {
            None => candidate_names.iter().cloned().collect(),
            Some(set) => candidate_names
                .iter()
                .filter(|name| set.contains(*name))
                .cloned()
                .collect(),
        };
        let excluded: Vec<String> = candidate_names
            .iter()
            .filter(|name| !registered.contains(*name))
            .cloned()
            .collect();
        let selection = ContextualToolSelectionMiddleware::inheriting(
            Some(candidate_names.to_vec()),
            Vec::<String>::new(),
            Some(registered.iter().cloned().collect::<Vec<_>>()),
            excluded,
        );
        let allowlist = ToolAllowlistMiddleware::new(registered.iter().cloned());
        Self {
            allowlist,
            selection,
            registered,
            tags,
            candidate_count: candidate_names.len(),
            reported: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Middleware<()> for OpenHumanToolExposureMiddleware {
    fn name(&self) -> &str {
        "openhuman_tool_exposure"
    }

    async fn before_model(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        self.selection.before_model(ctx, state, request).await?;
        if !self.reported.swap(true, Ordering::SeqCst) {
            tracing::debug!(
                exposed = request.tools.len(),
                candidates = self.candidate_count,
                registered = self.registered.len(),
                tags = ?self.tags,
                "[tool-exposure] authoritative selection applied to live request"
            );
        }
        Ok(())
    }

    async fn before_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        self.allowlist.before_tool(ctx, state, call).await
    }
}

#[cfg(test)]
#[path = "tool_exposure_tests.rs"]
mod tool_exposure_tests;
