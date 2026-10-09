//! [`ToolOutcomeCaptureMiddleware`]: record each tool call's real outcome
//! (success, classified failure, final content) into the shared sink before
//! the harness folds it into a transcript message.

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};
use tinytools::ToolResult as TaToolResult;

/// `after_tool`: capture each tool call's execution outcome (success + content)
/// into a shared sink before the harness folds the result into a `Message::tool`
/// that drops the `error` flag (issue #4249). Without this, a post-turn
/// `ToolCallRecord` could only report every call as an optimistic success — the
/// in-house engine tracked real per-call success. The crate runs `after_tool` in
/// REVERSE registration order (issue #4464), so registering this AFTER the
/// summarization/cap middlewares (i.e. pushing it EARLIER, before
/// `TurnContextMiddleware::install`) makes its `after_tool` run AFTER those caps —
/// recording the final (summarized/capped) content the transcript keeps, not the
/// raw payload.
pub(crate) struct ToolOutcomeCaptureMiddleware {
    sink: crate::agent::tinyagents::ToolOutcomeSink,
    /// `call_id → (success, classified failure, elapsed, output chars)` fallback
    /// read by the event bridge when projecting `ToolCallCompleted`. TinyAgents
    /// 1.6 owns the raw outcome fields; the host still adds classified failure
    /// metadata for the UI.
    failure_map: crate::agent::tinyagents::observability::ToolFailureMap,
    /// Invocation start + the original structured arguments.  `after_tool`
    /// receives only an identity/result pair, so preserve both facts at the
    /// boundary where TinyTools still exposes the full call.
    started: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (std::time::Instant, serde_json::Value)>,
        >,
    >,
}

impl ToolOutcomeCaptureMiddleware {
    pub(crate) fn new(
        sink: crate::agent::tinyagents::ToolOutcomeSink,
        failure_map: crate::agent::tinyagents::observability::ToolFailureMap,
    ) -> Self {
        Self {
            sink,
            failure_map,
            started: Default::default(),
        }
    }
}

#[async_trait]
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for ToolOutcomeCaptureMiddleware
{
    fn name(&self) -> &str {
        "tool_outcome_capture"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &mut tinyinference_llm::tool::ToolCall,
    ) -> TaResult<()> {
        if let Ok(mut started) = self.started.lock() {
            started.insert(
                call.id.to_string(),
                (std::time::Instant::now(), call.arguments.clone()),
            );
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let tool_name = invocation.tool_name();
        let call_id = invocation.call_id().to_string();
        let (duration_ms, arguments) = self
            .started
            .lock()
            .ok()
            .and_then(|mut started| started.remove(&call_id))
            .map(|(started, arguments)| {
                (
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    arguments,
                )
            })
            .unwrap_or_default();
        // Enrich a raw security-policy / autonomy block (issue #4094): the ~20
        // `[policy-blocked]` denials emitted deep in `SecurityPolicy` / the tools
        // return a bare marker line with no workaround and no relay directive, so
        // the agent dead-ends. Rewrite the content into the structured
        // `Blocked / Reason / Workaround / relay` shape here — the last `after_tool`
        // hook, so the enriched text is what the transcript keeps. The marker is
        // preserved, and already-structured `ToolPolicyMiddleware` denials (which
        // carry a `Workaround:` suffix) are left untouched. This runs before
        // classification below, which still recognises the preserved marker.
        if let Some(enriched) = crate::agent::tinyagents::policy_denial::maybe_enrich_policy_block(
            tool_name,
            &crate::agent::tinyagents::middleware::tool_result_text(result),
        ) {
            tracing::debug!(
                tool = tool_name,
                "[tinyagents::mw] enriched raw security-policy block with workaround + relay"
            );
            crate::agent::tinyagents::middleware::replace_tool_result_text(result, enriched);
        }

        let success = !result.is_error;
        // Classify the failure so the live `ToolCallCompleted` event and the
        // persisted timeline can explain it in plain language. The classifier
        // owns all marker precedence now (policy-blocked / policy-denied / TTL
        // expiry short-circuit ahead of the `timed out` sniff — #4459), so this
        // just hands it the failure text.
        //
        // Sniff both `error` and `content`: the classifier historically read
        // `error` while the marker/timeout sniffs read `content`, a latent
        // asymmetry (#4459). Combine them so a marker/phrase is found wherever
        // the tool layer put it.
        let failure = if success {
            None
        } else {
            let combined = crate::agent::tinyagents::middleware::tool_result_text(result);
            let timed_out = combined.contains("timed out");
            Some(crate::tools::status::classify(&combined, timed_out))
        };
        // Host-only structured payload (e.g. `{"kind":"web_search", ...}`) a
        // tool attached via `ToolResult::metadata` for a richer UI
        // presentation than plain text allows. Only forwarded when it is a
        // JSON object carrying a `"kind"` discriminator, so an arbitrary
        // metadata shape a tool sets for its own bookkeeping doesn't leak onto
        // the wire as if it were a presentation contract.
        if result
            .metadata
            .as_ref()
            .and_then(|v| v.get("kind"))
            .and_then(serde_json::Value::as_str)
            == Some("mcp_result")
        {
            tracing::debug!(
                tool = tool_name,
                "[tinyagents::mw] dropped an undecorated mcp_result envelope"
            );
            result.metadata = None;
        }
        let structured = result
            .metadata
            .clone()
            .filter(|v| v.is_object() && v.get("kind").is_some());
        if let Ok(mut map) = self.failure_map.lock() {
            // Keep duration + rendered output size as a compatibility fallback
            // for old/deserialized completion events; TinyAgents 1.6 supplies
            // these fields directly on live `ToolCompleted` events.
            map.insert(
                call_id.clone(),
                (
                    success,
                    failure,
                    duration_ms,
                    crate::agent::tinyagents::middleware::tool_result_text(result)
                        .chars()
                        .count(),
                    structured,
                ),
            );
        }
        if let Ok(mut sink) = self.sink.lock() {
            sink.push(crate::agent::tinyagents::ToolCallOutcome {
                call_id,
                name: tool_name.to_string(),
                arguments,
                success,
                content: crate::agent::tinyagents::middleware::tool_result_text(result),
                duration_ms,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tool_outcome_capture_tests.rs"]
mod tests;
