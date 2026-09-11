// `CredentialScrubMiddleware` and `ToolPolicyMiddleware`, split out of
// `middleware_part_02.rs` when that file crossed the 750-line layout limit
// (`scripts/ci/check-openhuman-rust-layout.mjs`). Both are `wrap_tool` hooks
// registered by `assemble_turn_harness`; nothing about the split is semantic,
// and `middleware.rs` includes the parts in order so the module contents are
// unchanged.

/// `wrap_tool`: scrub credential-shaped secrets out of every tool result before
/// it leaves the tool boundary (issue #4453). The legacy engine ran
/// `scrub_credentials` over **every** tool output before it entered model
/// context (`engine/tools.rs`); the tinyagents path dropped that call site, so
/// secrets in tool output (env dumps, config reads, API responses, shell output)
/// reached model context, on-disk `session_raw` transcripts, worker-thread
/// mirrors, and the tool-outcome capture sink — violating "Never log secrets or
/// full PII".
///
/// Installed as the **innermost** tool wrap (pushed last), so it observes the
/// RAW tool result first and scrubs it before any outer wrap, the `after_tool`
/// chain (summarization/caps in [`ToolOutputMiddleware`]), the transcript push,
/// or the [`ToolOutcomeCaptureMiddleware`] sink can see the unredacted content.
/// Scrubbing here — rather than inside `execute_openhuman_tool` — covers the
/// parent chat path, sub-agent paths, the persisted transcript, and
/// `ToolCallOutcome` records by construction, since every path runs the same
/// `assemble_turn_harness` seam.
pub(super) struct CredentialScrubMiddleware;

impl CredentialScrubMiddleware {
    pub(super) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolMiddleware<()> for CredentialScrubMiddleware {
    fn name(&self) -> &str {
        "credential_scrub"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        let tool_name = call.name.clone();
        let outcome = next.run(ctx, state, call).await?;
        // `MiddlewareToolOutcome` is `#[non_exhaustive]`; today it only carries a
        // `Result`, but match rather than irrefutable-let so a future variant
        // fails loud instead of silently bypassing scrubbing.
        let mut result = match outcome {
            MiddlewareToolOutcome::Result(result) => result,
            other => return Ok(other),
        };

        let scrubbed_content =
            crate::openhuman::agent::harness::credentials::scrub_credentials(&result.content);
        if scrubbed_content != result.content {
            tracing::warn!(
                tool = %tool_name,
                "[tinyagents::mw] credential_scrub redacted secret(s) from tool result content"
            );
            result.content = scrubbed_content;
        }

        if let Some(err) = result.error.as_ref() {
            let scrubbed_err =
                crate::openhuman::agent::harness::credentials::scrub_credentials(err);
            if &scrubbed_err != err {
                tracing::warn!(
                    tool = %tool_name,
                    "[tinyagents::mw] credential_scrub redacted secret(s) from tool result error"
                );
                result.error = Some(scrubbed_err);
            }
        }

        // Raw JSON payloads (rarely populated on this path) can carry the same
        // secrets — walk their string leaves so a scrubbed `content` isn't
        // undermined by an unredacted `raw` mirror.
        if let Some(raw) = result.raw.take() {
            result.raw = Some(scrub_json_credentials(raw));
        }

        Ok(MiddlewareToolOutcome::Result(result))
    }
}

/// Recursively scrub credential-shaped string leaves inside a JSON value.
fn scrub_json_credentials(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => {
            Value::String(crate::openhuman::agent::harness::credentials::scrub_credentials(&s))
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(scrub_json_credentials).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, scrub_json_credentials(v)))
                .collect(),
        ),
        other => other,
    }
}

/// `wrap_tool`: enforce the agent's builder-configured [`ToolPolicy`] at the tool
/// boundary (issue #4249). The in-house engine ran this check in
/// `agent_tool_exec` (`ctx.tool_policy.check(...)`); the tinyagents path bypassed
/// it, so a `.tool_policy()` deny/require-approval silently no-opped and the tool
/// executed anyway — a security regression. This middleware restores it: a
/// blocking decision short-circuits with a model-consumable result carrying the
/// same `"Tool '<name>' <denied|requires approval> by policy '<policy>': <reason>"`
/// wording the engine produced.
pub(super) struct ToolPolicyMiddleware {
    policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel deny
    /// + per-call permission-level ceiling the engine ran in `agent_tool_exec`.
    session: crate::openhuman::tools::agent_policy::ToolPolicySession,
    /// Shared tool sets (same `Arc`s the runner registers) so a call's OpenHuman
    /// `Tool` can be resolved for its generated-tool runtime context and its
    /// per-call permission level.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    session_id: String,
    channel: String,
    agent_definition_id: String,
}

impl ToolPolicyMiddleware {
    pub(super) fn new(
        policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
        session: crate::openhuman::tools::agent_policy::ToolPolicySession,
        tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
        session_id: String,
        channel: String,
        agent_definition_id: String,
    ) -> Self {
        Self {
            policy,
            session,
            tool_sets,
            session_id,
            channel,
            agent_definition_id,
        }
    }

    fn resolve_tool(&self, name: &str) -> Option<&Box<dyn Tool>> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
    }

    /// The delegation tools this session can actually call that reach one of
    /// `owners`, as tool names.
    ///
    /// Derived from the session's own tool set — every synthesised `delegate_*`
    /// tool publishes its target agent on the erased host-extension slot
    /// (`traits::delegation_target`). That is deliberately the only source: a
    /// static owner-to-tool table would duplicate each agent's `delegate_name`
    /// and could name a tool this session was never built with, sending the
    /// model from one dead end into another. Asking the tool set cannot.
    fn callable_delegates_for(&self, owners: &[&str]) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for tool in self.tool_sets.iter().flat_map(|set| set.iter()) {
            let Some(target) =
                crate::openhuman::tools::traits::delegation_target(tool.as_ref())
            else {
                continue;
            };
            if !owners.contains(&target) {
                continue;
            }
            let name = tool.name().to_string();
            // Uses `is_denied()`, and that is deliberate — it is the same
            // predicate as the gate this hint points at.
            //
            // Spelled out, because two predicates live in this file and a
            // sentence that does not name one has been misread three times:
            //
            //   * This hint names a tool for the model to call DIRECTLY.
            //   * The direct-call gate is `channel_permission_block`'s first
            //     check, `if decision.is_denied()` (this file, top of the fn).
            //   * `is_denied()` is `!matches!(action, Allow)`, so it is TRUE for
            //     `HideFromPrompt` — that check is what refuses a prompt-hidden
            //     tool called by name.
            //   * Therefore a prompt-hidden delegate is not a route, and
            //     `is_denied()` here is exactly what keeps it out.
            //
            // `blocks_execution()` would be wrong here: it deliberately admits
            // `HideFromPrompt` for the `use_skill` path below, where hiding is
            // the disclosure mechanism rather than a refusal. Same tool, two
            // call paths, two answers. A hint must use the predicate of the gate
            // it points at — the hint and the gate disagreeing is how this whole
            // class of bug started.
            //
            // Pinned by `a_prompt_hidden_delegate_is_not_offered_as_a_direct_route`.
            if self.session.decision_for(&name).is_denied() || found.contains(&name) {
                continue;
            }
            found.push(name);
        }
        found
    }

    /// The route sentence for a pack, resolved against THIS session.
    fn route_for_pack(&self, pack: &crate::openhuman::tools::toolpacks::ToolPack) -> String {
        crate::openhuman::tools::toolpacks::route_sentence(
            &self.callable_delegates_for(pack.owners),
            pack.owners,
        )
    }

    /// Render a `use_skill` listing scoped to what this session may call.
    ///
    /// This lives in the middleware because the middleware is the only layer
    /// that holds the session — `UseSkillTool` is built once per registry and
    /// has no idea who is calling it. Only the disclosure half is rendered here:
    /// a call that names a `tool` is the execution half, which
    /// `channel_permission_block` has already gated and the tool's own
    /// `execute` dispatches. Returns `None` when there is nothing to scope (a
    /// tool named, no `skill` argument, no pack handle), so the call falls
    /// through to the tool's own `execute` unchanged.
    fn render_skill_for_session(&self, call: &TaToolCall) -> Option<TaToolResult> {
        if crate::openhuman::tools::toolpacks::named_tool(&call.arguments).is_some() {
            return None;
        }
        let skill = call
            .arguments
            .get("skill")
            .and_then(serde_json::Value::as_str)?;
        let tool = self.resolve_tool(&call.name)?;
        let handle = crate::openhuman::tools::traits::pack_registry_handle(tool.as_ref())?;
        let is_callable = |name: &str| !self.session.decision_for(name).blocks_execution();
        let route = crate::openhuman::tools::toolpacks::pack(skill)
            .map(|pack| self.route_for_pack(pack))
            .unwrap_or_default();
        let rendered = crate::openhuman::tools::toolpacks::render_pack_filtered(
            skill,
            handle,
            // The same predicate the gate applies to `use_skill`'s inner tool.
            // Two sources of truth for "can this session call it" is the bug.
            &is_callable,
            &route,
        );
        let (content, error) = match rendered {
            Ok(text) => (text, None),
            Err(message) => (message.clone(), Some(message)),
        };
        Some(TaToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            raw: None,
            error,
            elapsed_ms: 0,
        })
    }

    /// The channel-permission gate the engine ran before the builder policy: a
    /// session-level deny, then a per-call permission-level ceiling check. Returns
    /// the blocking message when the call must not execute.
    fn channel_permission_block(&self, call: &TaToolCall) -> Option<String> {
        let decision = self.session.decision_for(&call.name);
        if decision.is_denied() {
            return Some(
                PolicyDenial::SessionForbidden {
                    tool: &call.name,
                    required: decision.required_permission,
                    allowed: decision.allowed_permission,
                    channel: &self.channel,
                }
                .render(),
            );
        }
        let tool = self.resolve_tool(&call.name)?;
        let call_required = tool.permission_level_with_args(&call.arguments);
        if call_required > decision.allowed_permission {
            return Some(
                PolicyDenial::PermissionTooLow {
                    tool: &call.name,
                    required: call_required,
                    allowed: decision.allowed_permission,
                    channel: &self.channel,
                }
                .render(),
            );
        }
        // For `use_skill`, also validate the resolved inner tool against the
        // session allowlist. Role-hidden packed tools are not checked by the
        // outer policy name; without this check `use_skill` would bypass the
        // session's effective allowlist for any packed tool.
        if call.name == "use_skill" {
            if let Some(inner_tool) = call
                .arguments
                .get("tool")
                .and_then(serde_json::Value::as_str)
            {
                // `blocks_execution`, NOT `is_denied`. Every withheld packed
                // tool is `HideFromPrompt`, and `use_skill` is the only route it
                // has — gating that route on `is_denied` refused all of them.
                let inner_decision = self.session.decision_for(inner_tool);
                if inner_decision.blocks_execution() {
                    // Name the route. A bare denial gives the model nothing to
                    // do differently, and a model with no next step retries the
                    // same call: one live turn burned its whole budget on six
                    // identical `use_skill` denials and died on the
                    // repeated-failure breaker. Same sentence the listing uses,
                    // resolved against the same session, so the two cannot
                    // tell the model different stories.
                    let hint = crate::openhuman::tools::toolpacks::pack_for_tool(inner_tool)
                        .map(|pack| self.route_for_pack(pack))
                        .filter(|h| !h.is_empty())
                        .map(|h| format!(" {h}"))
                        .unwrap_or_default();
                    return Some(format!(
                        "Tool `{inner_tool}` is not allowed in the current session and cannot be used through `use_skill`.{hint}"
                    ));
                }
            }
        }
        None
    }

    fn generated_context(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<crate::openhuman::agent::tool_policy::GeneratedToolRuntimeContext> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .and_then(|t| {
                crate::openhuman::tools::traits::generated_runtime_context(t.as_ref(), args)
            })
    }
}
