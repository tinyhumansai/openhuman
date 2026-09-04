
#[async_trait]
impl Middleware<()> for ToolOutputMiddleware {
    fn name(&self) -> &str {
        "tool_output_budget"
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // Proposal-/persistence-emitting workflow tools return a self-describing
        // `{ "type": "workflow_proposal", … }` JSON payload that `flows::ops`'
        // `extract_workflow_proposal` (and the frontend's content-based
        // recognition) parse structurally. Sampling tools (`get_tool_contract` /
        // `get_tool_output_sample`) return a real API response the model reads
        // to derive an exact array path/schema. All four stages below are
        // content-*rewriting*: tokenjuice (steps 1+2) tabulates any uniform
        // object-array of ≥3 rows over ~512 bytes into a `[json table: …]`
        // marker (stripping the `"type"` field on graphs with enough nodes, or
        // eliding the array a sample exists to reveal); the char cap and shared
        // byte-budget backstop (steps 3+4) truncate at a UTF-8 boundary, which
        // breaks the whole-string JSON parse both proposal consumers do. See
        // [`is_compaction_exempt`]/[`is_truncation_exempt`] for which stages
        // each tool family skips and why.
        let compaction_exempt = is_compaction_exempt(&result.name);
        let truncation_exempt = is_truncation_exempt(&result.name);
        if compaction_exempt {
            tracing::debug!(
                tool = %result.name,
                bytes = result.content.len(),
                "[tinyagents::mw] compaction-exempt: skipping payload summarizer + tokenjuice"
            );
        }
        if truncation_exempt {
            tracing::debug!(
                tool = %result.name,
                bytes = result.content.len(),
                "[tinyagents::mw] truncation-exempt: skipping per-tool char cap + shared byte-budget backstop"
            );
        }

        // 1. Semantic summarization (progressive disclosure) — swap the raw
        //    payload for a compressed summary when the summarizer opts in.
        //    Failures never break the tool call, but they are no longer
        //    silent: when summarization does not happen the model is told so
        //    in the payload itself. This used to be
        //    `if let Ok(Some(payload)) = …`, which discarded `Err(_)` and
        //    `Ok(None)` identically — so a failed summarization reached the
        //    model as an unannounced raw dump and it re-called the same tool.
        // Held until after the caps below rather than prefixed here. The notice
        // is ~165 chars; a tool declaring a `max_result_size_chars` smaller than
        // that had step 3 run `chars().take(cap)` straight through it, cutting
        // the reason text and the do-not-re-run sentence mid-word — so the one
        // stage that exists to stop a re-dispatch loop was removed exactly when
        // the output was most aggressively truncated. Capping the payload first
        // and prefixing afterwards also means a tool's declared cap bounds the
        // tool's own output, which is what it is a contract about, rather than
        // openhuman's annotation about it.
        let mut pending_notice: Option<&'static str> = None;

        if !compaction_exempt {
            if let Some(ps) = &self.payload_summarizer {
                match ps
                    .maybe_summarize_in_parent(ctx, &result.name, None, &result.content)
                    .await
                {
                    Ok(SummarizeOutcome::Summarized(payload)) => {
                        tracing::info!(
                            tool = %result.name,
                            from_bytes = payload.original_bytes,
                            to_bytes = payload.summary_bytes,
                            "[tinyagents::mw] payload_summarizer compressed tool output"
                        );
                        ctx.emit(AgentEvent::Compressed {
                            from_tokens: estimate_output_tokens(payload.original_bytes),
                            to_tokens: estimate_output_tokens(payload.summary_bytes),
                        });
                        result.content = payload.summary;
                    }
                    // The payload was fine as it was. Say nothing: a notice on
                    // every small tool result would be pure noise.
                    Ok(SummarizeOutcome::NotNeeded) => {}
                    Ok(SummarizeOutcome::Unavailable(reason)) => {
                        tracing::warn!(
                            tool = %result.name,
                            bytes = result.content.len(),
                            ?reason,
                            "[tinyagents::mw] payload_summarizer unavailable; disclosing raw output"
                        );
                        pending_notice = Some(reason.notice());
                    }
                    // Reserved for fatal misconfiguration. Previously
                    // indistinguishable from "nothing to do"; the model is now
                    // told the output is raw for the same reason as above.
                    Err(error) => {
                        tracing::warn!(
                            tool = %result.name,
                            bytes = result.content.len(),
                            error = %error,
                            "[tinyagents::mw] payload_summarizer errored; disclosing raw output"
                        );
                        pending_notice = Some(UnavailableReason::Failed.notice());
                    }
                }
            }

            // 2. TokenJuice content-aware compaction. This mirrors the legacy
            //    `agent_tool_exec` stage that ran after semantic summarization and
            //    before the hard output caps.
            let before_tokenjuice_bytes = result.content.len();
            let compacted = crate::openhuman::inference::tokenjuice::compact_output_with_policy(
                std::mem::take(&mut result.content),
                &result.name,
                self.tokenjuice_compaction_enabled,
                self.tokenjuice_compression,
            )
            .await;
            result.content = compacted;
            let after_tokenjuice_bytes = result.content.len();
            if after_tokenjuice_bytes < before_tokenjuice_bytes {
                ctx.emit(AgentEvent::Compressed {
                    from_tokens: estimate_output_tokens(before_tokenjuice_bytes),
                    to_tokens: estimate_output_tokens(after_tokenjuice_bytes),
                });
            }
        }

        // 3. Per-tool **char** cap — a tool that declares `max_result_size_chars`
        //    caps its own output in characters, with the tool-cap marker the model
        //    was taught to read (legacy engine parity). Distinct from the generic
        //    byte budget below: the tool cap is the tool's own contract. Skipped
        //    for truncation-exempt tools (see [`is_truncation_exempt`]) — the tool
        //    cap is still *computed* below (step 4's "no cap of its own" check
        //    reads it), just not applied to `result.content`.
        let tool_cap = self.tool_char_cap(&result.name);
        if !truncation_exempt {
            if let Some(cap) = tool_cap {
                let char_count = result.content.chars().count();
                if char_count > cap {
                    let truncated: String = result.content.chars().take(cap).collect();
                    let dropped = char_count - cap;
                    tracing::debug!(
                        tool = %result.name,
                        cap,
                        char_count,
                        dropped,
                        "[tinyagents::mw] per-tool char cap applied"
                    );
                    result.content = format!(
                        "{truncated}\n\n[truncated by tool cap: {dropped} more chars not shown]"
                    );
                }
            }
        }

        // 4. Shared byte-cap backstop — truncate at a UTF-8 boundary with a marker.
        //    Only for tools with no cap of their own (a capped tool already bounded
        //    itself above; stacking the two markers would double-truncate), and
        //    never for truncation-exempt tools. This is a per-result cap only —
        //    `apply_per_result_persistence` takes a single `content: String` and a
        //    fixed `self.budget_bytes`, with no shared/global accumulator across
        //    tool calls (the aggregate-spill variant, `spill_aggregate_tool_results`,
        //    is a separate legacy code path not wired into this middleware) — so
        //    exempting these tools' own contribution here cannot perturb any other
        //    tool's budget accounting.
        if !truncation_exempt && tool_cap.is_none() && self.budget_bytes > 0 {
            let (capped, outcome) = apply_per_result_persistence(
                std::mem::take(&mut result.content),
                self.artifact_store.as_ref(),
                &result.name,
                Some(&result.call_id),
                self.budget_bytes,
            )
            .await;
            if outcome.persisted {
                tracing::info!(
                    tool = %result.name,
                    from_bytes = outcome.original_bytes,
                    to_bytes = outcome.final_bytes,
                    "[tinyagents::mw] tool_result_artifact persisted oversized output"
                );
                if let Some(path) = outcome.artifact_path.as_deref() {
                    if let Some(store) = ctx.stores.get(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE) {
                        let key = result.call_id.clone();
                        let mut fields = serde_json::Map::new();
                        fields.insert("tool".to_string(), result.name.clone().into());
                        fields.insert("call_id".to_string(), result.call_id.clone().into());
                        fields.insert("artifact_path".to_string(), path.to_string().into());
                        fields.insert(
                            "original_bytes".to_string(),
                            serde_json::Value::from(outcome.original_bytes as u64),
                        );
                        fields.insert(
                            "preview_bytes".to_string(),
                            serde_json::Value::from(outcome.final_bytes as u64),
                        );
                        let index_result: tinyagents_harness::Result<()> =
                            store.put("tool_results", &key, fields.into()).await;
                        if let Err(err) = index_result {
                            tracing::warn!(
                                tool = %result.name,
                                call_id = %result.call_id,
                                error = %err,
                                "[tinyagents::mw] failed to index tool_result_artifact"
                            );
                        } else {
                            tracing::debug!(
                                tool = %result.name,
                                call_id = %result.call_id,
                                artifact_path = %path,
                                "[tinyagents::mw] indexed tool_result_artifact in run store"
                            );
                        }
                    }
                }
            } else if outcome.original_bytes != outcome.final_bytes {
                tracing::debug!(
                    tool = %result.name,
                    from_bytes = outcome.original_bytes,
                    to_bytes = outcome.final_bytes,
                    "[tinyagents::mw] tool_result_budget truncated tool output"
                );
            }
            result.content = capped;
        }

        // 5. The disclosure, last, so no cap above can eat it. The model has to
        //    be able to read *why* the payload is raw and that re-running will
        //    not summarize it — a half-truncated notice is worse than none,
        //    because it still looks like tool output.
        if let Some(notice) = pending_notice {
            result.content = format!("{notice}\n\n{}", result.content);
        }

        Ok(())
    }
}

/// `wrap_tool`: route OpenHuman's human-in-the-loop **approval gate** through a
/// named tinyagents tool middleware (issue #4249, Phase 1). A tool with an
/// external effect intercepts through the global [`ApprovalGate`]; a denial
/// short-circuits with the reason as a model-consumable [`TaToolResult`]
/// (`next` is never called), and an allowed call records a terminal audit row
/// once the tool resolves.
///
/// This replaces the inline approval block that used to live in
/// `execute_openhuman_tool`, giving approval a stable middleware name and
/// letting it short-circuit cleanly. Tool-*internal* security (path/command
/// policy via `live_policy`) stays inside each tool — it needs tool-specific
/// operation semantics the harness boundary can't reconstruct generically.
const COMPOSIO_EXECUTE_TOOL: &str = "composio_execute";
const INVALID_COMPOSIO_APPROVAL_NAME: &str = "composio_execute:<invalid-action>";

/// Stable identity used by persistent approval grants.
///
/// `composio_execute` multiplexes every Composio action through one outer tool
/// name. Keying "Always allow" by that name would let approval for one action
/// authorize every later action, so use the namespaced action slug instead.
fn approval_tool_name<'a>(
    tool_name: &'a str,
    args: &'a serde_json::Value,
) -> std::borrow::Cow<'a, str> {
    if tool_name != COMPOSIO_EXECUTE_TOOL {
        return std::borrow::Cow::Borrowed(tool_name);
    }
    let slug = args
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|slug| !slug.is_empty());
    match slug {
        Some(slug) => std::borrow::Cow::Owned(format!("{COMPOSIO_EXECUTE_TOOL}:{slug}")),
        None => std::borrow::Cow::Borrowed(INVALID_COMPOSIO_APPROVAL_NAME),
    }
}

pub(super) struct ApprovalSecurityMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's OpenHuman `Tool` by name so `external_effect_with_args` can gate.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(super) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    /// Whether the named tool declares an external effect for these args.
    fn has_external_effect(&self, name: &str, args: &serde_json::Value) -> bool {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.external_effect_with_args(args))
            .unwrap_or(false)
    }
}

#[async_trait]
impl ToolMiddleware<()> for ApprovalSecurityMiddleware {
    fn name(&self) -> &str {
        "approval_security"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        // Resolve external-effect up front so no tool borrow is held across the
        // approval await.
        let mut audit_id: Option<String> = None;
        let has_ext = self.has_external_effect(&call.name, &call.arguments);
        tracing::debug!(
            tool = %call.name,
            has_external_effect = has_ext,
            "[tinyagents::mw] checking tool for approval"
        );
        if has_ext {
            if let Some(gate) = ApprovalGate::try_global() {
                let approval_name = approval_tool_name(&call.name, &call.arguments);
                tracing::debug!(
                    tool = %call.name,
                    approval_name = %approval_name,
                    "[tinyagents::mw] routing external-effect tool through approval gate"
                );
                let summary = summarize_action(&call.name, &call.arguments);
                let redacted = redact_args(&call.arguments);
                let (outcome, request_id) =
                    gate.intercept_audited(approval_name.as_ref(), &summary, redacted).await;
                match outcome {
                    GateOutcome::Deny { reason } => {
                        tracing::warn!(
                            tool = %call.name,
                            reason = %reason,
                            "[tinyagents::mw] approval gate denied tool call"
                        );
                        return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                            call_id: call.id,
                            name: call.name,
                            content: reason.clone(),
                            raw: None,
                            error: Some(reason),
                            elapsed_ms: 0,
                        }));
                    }
                    GateOutcome::Allow => audit_id = request_id,
                }
            } else {
                tracing::warn!(
                    tool = %call.name,
                    "[tinyagents::mw] approval gate unavailable; external-effect tool will run without interactive approval"
                );
            }
        }

        let outcome = next.run(ctx, state, call).await?;

        // Record the terminal audit row for an approved external-effect call
        // (idempotent; a no-op when the id is unknown).
        if let Some(id) = audit_id {
            if let Some(gate) = ApprovalGate::try_global() {
                if let MiddlewareToolOutcome::Result(res) = &outcome {
                    let exec = if res.error.is_some() {
                        ExecutionOutcome::Failure
                    } else {
                        ExecutionOutcome::Success
                    };
                    gate.record_execution(&id, exec, res.error.as_deref());
                }
            }
        }
        Ok(outcome)
    }
}

/// `wrap_tool`: refuse a tool whose scope is
/// [`ToolScope::CliRpcOnly`](crate::openhuman::tools::ToolScope) inside the
/// autonomous agent loop (issue #4249). The in-house engine ran this gate in
/// `engine::tools`; the tinyagents path dropped it, so a CLI/RPC-only tool
/// (e.g. phone calls) would execute from the model loop. Applies on every path
/// (channel, session, sub-agent) since the restriction is intrinsic to the tool,
/// not the session — installed unconditionally.
pub(super) struct CliRpcOnlyMiddleware {
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl CliRpcOnlyMiddleware {
    pub(super) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    fn is_cli_rpc_only(&self, name: &str) -> bool {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.scope() == crate::openhuman::tools::ToolScope::CliRpcOnly)
            .unwrap_or(false)
    }
}

#[async_trait]
impl ToolMiddleware<()> for CliRpcOnlyMiddleware {
    fn name(&self) -> &str {
        "cli_rpc_only"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        if self.is_cli_rpc_only(&call.name) {
            tracing::warn!(
                tool = call.name.as_str(),
                "[tinyagents::mw] tool scope is CliRpcOnly — denied in agent loop"
            );
            let content = format!(
                "Tool '{}' is only available via explicit CLI/RPC invocation, not in the autonomous agent loop.",
                call.name
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: content.clone(),
                raw: None,
                error: Some(content),
                elapsed_ms: 0,
            }));
        }
        next.run(ctx, state, call).await
    }
}

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
                let inner_decision = self.session.decision_for(inner_tool);
                if inner_decision.is_denied() {
                    return Some(format!(
                        "Tool `{inner_tool}` is not allowed in the current session and cannot be used through `use_skill`."
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
