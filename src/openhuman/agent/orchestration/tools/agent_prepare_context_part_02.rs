
#[async_trait]
impl Tool for AgentPrepareContextTool {
    fn name(&self) -> &str {
        "agent_prepare_context"
    }

    fn description(&self) -> &str {
        "Before answering or delegating, scout existing context. Runs a fast \
         read-only context-collector that checks memory, past conversations \
         (transcripts), your goals/profile, installed/registry skills, connected \
         integrations, and the web, then returns whether there's enough context \
         to answer, a compact context summary, an ordered list of recommended \
         next tool calls (parent tools, by exact name, with args), and any \
         skills worth running. Use only when a caller explicitly needs an \
         ad hoc scout pass. If the current prompt says agent context has \
         already been prepared, use the prepared context and do not call this \
         tool again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["question"],
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The user's request or goal to gather context for. Be specific — the scout has no memory of your conversation."
                },
                "focus": {
                    "type": "string",
                    "description": "Optional hint that narrows what to scout (e.g. a platform, time window, or sub-question)."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // ReadOnly, not Execute: this tool only ever runs the read-only
        // `context_scout` (read_only sandbox, no write/exec tools). Marking it
        // Execute would make `ToolPolicyEngine` strip it from any
        // provider-visible set on a `ReadOnly`-capped channel, which would hide
        // the scout from callers that still expose it explicitly.
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let prepared_sources = current_agent_context_prepared_sources();
        if !prepared_sources.is_empty() {
            tracing::info!(
                target: "agent_prepare_context",
                sources = ?prepared_sources,
                "[agent_prepare_context] skipped because agent context is already prepared for this turn"
            );
            return Ok(ToolResult::success(already_prepared_context_bundle(
                &prepared_sources,
            )));
        }

        let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
        let focus = args.get("focus").and_then(|v| v.as_str());
        let tool_catalog = AgentPrepareContextTool::render_parent_tool_catalog();
        run_context_scout_with_catalog_and_workspace(
            question,
            focus,
            &tool_catalog,
            tool_context.and_then(|ctx| ctx.workspace().cloned()),
        )
        .await
    }
}
