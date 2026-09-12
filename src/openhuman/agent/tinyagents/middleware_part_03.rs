
#[async_trait]
impl ToolMiddleware<()> for ToolPolicyMiddleware {
    fn name(&self) -> &str {
        "tool_policy"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        use crate::openhuman::agent::tool_policy::{
            ToolCallContext, ToolPolicyDecision, ToolPolicyRequest,
        };

        // Channel-permission ceiling first (session deny + per-call permission
        // level), mirroring the engine order in `agent_tool_exec`.
        if let Some(message) = self.channel_permission_block(&call) {
            tracing::debug!(
                tool = call.name.as_str(),
                channel = self.channel.as_str(),
                "[tinyagents::mw] tool blocked by channel permission ceiling"
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: message.clone(),
                raw: None,
                error: Some(message),
                elapsed_ms: 0,
            }));
        }

        let context = ToolCallContext::session(
            self.session_id.clone(),
            self.channel.clone(),
            self.agent_definition_id.clone(),
            call.id.clone(),
            1,
        );
        let mut request =
            ToolPolicyRequest::new(call.name.clone(), call.arguments.clone(), context);
        if let Some(generated) = self.generated_context(&call.name, &call.arguments) {
            request = request.with_generated_tool_context(generated);
        }

        let decision = self.policy.check(&request).await;
        if let Some(reason) = decision.blocking_reason() {
            let blocked_action = match &decision {
                ToolPolicyDecision::RequireApproval { .. } => "requires approval",
                ToolPolicyDecision::Deny { .. } => "denied",
                ToolPolicyDecision::Allow => "allowed",
            };
            crate::openhuman::tools::registry::denials::record(
                call.name.as_str(),
                self.policy.name(),
                blocked_action,
                reason,
            );
            tracing::debug!(
                tool = call.name.as_str(),
                policy = self.policy.name(),
                action = blocked_action,
                reason = %reason,
                "[tinyagents::mw] tool blocked by policy"
            );
            let content = match &decision {
                ToolPolicyDecision::RequireApproval { .. } => PolicyDenial::ApprovalRequired {
                    tool: &call.name,
                    policy: self.policy.name(),
                    reason,
                },
                _ => PolicyDenial::PolicyDenied {
                    tool: &call.name,
                    policy: self.policy.name(),
                    reason,
                },
            }
            .render();
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: content.clone(),
                raw: None,
                error: Some(content),
                elapsed_ms: 0,
            }));
        }

        // `use_skill`'s disclosure half (a `skill` with no `tool`) renders its
        // listing here, not in its own `execute`, because only the middleware
        // holds the session — and a listing that disagrees with the session is
        // a menu the model cannot order from. The execution half falls through:
        // `render_skill_for_session` answers `None` for it.
        //
        // Placed AFTER both gates on purpose. Rendering before them would let a
        // `use_skill` that the session forbids, or that the policy denies or
        // holds for approval, still hand back a full pack listing — the gates
        // would be advisory for this one tool.
        if call.name == crate::openhuman::tools::toolpacks::USE_SKILL {
            if let Some(result) = self.render_skill_for_session(&call) {
                return Ok(MiddlewareToolOutcome::Result(result));
            }
        }

        next.run(ctx, state, call).await
    }
}

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
    sink: super::ToolOutcomeSink,
    /// `call_id → (success, classified failure, elapsed, output chars)` fallback
    /// read by the event bridge when projecting `ToolCallCompleted`. TinyAgents
    /// 1.6 owns the raw outcome fields; the host still adds classified failure
    /// metadata for the UI.
    failure_map: super::observability::ToolFailureMap,
}

/// Delivers tool lifecycle events to hooks installed by an embedding host.
pub(crate) struct EmbedderToolHooksMiddleware {
    hooks: Vec<std::sync::Arc<dyn crate::openhuman::agent::hooks::ToolHook>>,
    /// Normalized pre-call arguments keyed by provider `call_id`, so the
    /// `PostToolUse` context can hand an embedding host the same arguments its
    /// `PreToolUse` hook saw. The crate's `ToolResult` does not carry the
    /// original call arguments, so without this cache every post-use event would
    /// report `Null` and an auditing/correlating host could not match inputs to
    /// outcomes.
    arguments_by_call_id: std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>,
}

impl EmbedderToolHooksMiddleware {
    pub(crate) fn new(
        hooks: Vec<std::sync::Arc<dyn crate::openhuman::agent::hooks::ToolHook>>,
    ) -> Self {
        Self {
            hooks,
            arguments_by_call_id: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl Middleware<()> for EmbedderToolHooksMiddleware {
    fn name(&self) -> &str {
        "embedder_tool_hooks"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        let mut context = crate::openhuman::agent::hooks::ToolHookContext {
            event: crate::openhuman::agent::hooks::ToolHookEvent::PreToolUse,
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            success: None,
            duration_ms: None,
            output: None,
            error: None,
            session_id: None,
            agent_id: None,
        };
        for hook in &self.hooks {
            match hook.before_tool_decision(&context).await {
                crate::openhuman::agent::hooks::ToolHookDecision::Proceed => {}
                // A rewrite is applied to the live call *and* to the context
                // handed to later hooks, so a chain of hooks each narrowing the
                // arguments composes instead of the last one silently winning.
                crate::openhuman::agent::hooks::ToolHookDecision::ProceedWith(arguments) => {
                    tracing::debug!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        "[tinyagents::mw] tool hook rewrote call arguments"
                    );
                    call.arguments = arguments.clone();
                    context.arguments = arguments;
                }
                crate::openhuman::agent::hooks::ToolHookDecision::Deny(reason) => {
                    return Err(tinyagents_harness::error::TinyAgentsError::Tool(format!(
                        "tool hook '{}' denied {}: {reason}",
                        hook.name(),
                        context.tool_name
                    )));
                }
                // There is no approval channel inside a middleware, and a hook
                // that asks for a human is asking for something stricter than
                // "proceed" — so an unresolvable `Ask` denies rather than
                // quietly allowing. The approval-gate path in
                // `security::approval` is where an interactive host resolves it.
                crate::openhuman::agent::hooks::ToolHookDecision::Ask(reason) => {
                    tracing::info!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        "[tinyagents::mw] tool hook requested approval; denying in a \
                         non-interactive middleware"
                    );
                    return Err(tinyagents_harness::error::TinyAgentsError::Tool(format!(
                        "tool hook '{}' requires approval for {}: {reason}",
                        hook.name(),
                        context.tool_name
                    )));
                }
            }
        }
        // Cache the (already-recovered) arguments only once every hook approved
        // the call: a vetoed call never reaches `after_tool`, so storing it here
        // would leak a cache entry for the turn.
        self.arguments_by_call_id
            .lock()
            .expect("embedder tool-hook arguments poisoned")
            .insert(call.id.clone(), call.arguments.clone());
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let arguments = self
            .arguments_by_call_id
            .lock()
            .expect("embedder tool-hook arguments poisoned")
            .remove(&result.call_id)
            .unwrap_or(serde_json::Value::Null);
        let context = crate::openhuman::agent::hooks::ToolHookContext {
            event: crate::openhuman::agent::hooks::ToolHookEvent::PostToolUse,
            call_id: result.call_id.clone(),
            tool_name: result.name.clone(),
            arguments,
            success: Some(result.error.is_none()),
            duration_ms: Some(result.elapsed_ms),
            output: Some(result.content.clone()),
            error: result.error.clone(),
            session_id: None,
            agent_id: None,
        };
        for hook in &self.hooks {
            // Text a hook returns is appended to the result the model reads —
            // the seam a "you edited a file, here is the linter output" hook
            // needs. It is appended rather than substituted so a hook cannot
            // erase what the tool actually said.
            if let Some(additional) = hook.after_tool_context(&context).await {
                if !additional.trim().is_empty() {
                    tracing::debug!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        chars = additional.chars().count(),
                        "[tinyagents::mw] tool hook appended context to the result"
                    );
                    result.content.push_str("\n\n");
                    result.content.push_str(additional.trim_end());
                }
            }
        }
        Ok(())
    }
}

impl ToolOutcomeCaptureMiddleware {
    pub(crate) fn new(
        sink: super::ToolOutcomeSink,
        failure_map: super::observability::ToolFailureMap,
    ) -> Self {
        Self { sink, failure_map }
    }
}

#[async_trait]
impl Middleware<()> for ToolOutcomeCaptureMiddleware {
    fn name(&self) -> &str {
        "tool_outcome_capture"
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // Enrich a raw security-policy / autonomy block (issue #4094): the ~20
        // `[policy-blocked]` denials emitted deep in `SecurityPolicy` / the tools
        // return a bare marker line with no workaround and no relay directive, so
        // the agent dead-ends. Rewrite the content into the structured
        // `Blocked / Reason / Workaround / relay` shape here — the last `after_tool`
        // hook, so the enriched text is what the transcript keeps. The marker is
        // preserved, and already-structured `ToolPolicyMiddleware` denials (which
        // carry a `Workaround:` suffix) are left untouched. This runs before
        // classification below, which still recognises the preserved marker.
        if let Some(enriched) =
            super::policy_denial::maybe_enrich_policy_block(&result.name, &result.content)
        {
            tracing::debug!(
                tool = result.name.as_str(),
                "[tinyagents::mw] enriched raw security-policy block with workaround + relay"
            );
            result.content = enriched;
        }

        let success = result.error.is_none();
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
            let error = result.error.as_deref().unwrap_or("");
            let combined: std::borrow::Cow<'_, str> = if error.is_empty() {
                std::borrow::Cow::Borrowed(result.content.as_str())
            } else if result.content.is_empty() || result.content == error {
                std::borrow::Cow::Borrowed(error)
            } else {
                std::borrow::Cow::Owned(format!("{error}\n{}", result.content))
            };
            let timed_out = combined.contains("timed out");
            Some(crate::openhuman::tools::status::classify(
                &combined, timed_out,
            ))
        };
        if let Ok(mut map) = self.failure_map.lock() {
            // Keep duration + rendered output size as a compatibility fallback
            // for old/deserialized completion events; TinyAgents 1.6 supplies
            // these fields directly on live `ToolCompleted` events.
            map.insert(
                result.call_id.clone(),
                (
                    success,
                    failure,
                    result.elapsed_ms,
                    result.content.chars().count(),
                ),
            );
        }
        if let Ok(mut sink) = self.sink.lock() {
            sink.push(super::ToolCallOutcome {
                call_id: result.call_id.clone(),
                name: result.name.clone(),
                success,
                content: result.content.clone(),
            });
        }
        Ok(())
    }
}

/// `before_tool`: repair a tool call's arguments *before* the harness runs its
/// fatal pre-execution schema gate (issues #4249 / #4451). A model can emit
/// arguments the model adapter parses to a non-object `Value` — invalid JSON
/// decodes to `Value::Null`, and some providers emit the whole arguments blob as
/// a JSON-encoded *string* (optionally wrapped in a ```json markdown fence). Left
/// alone the harness rejects those against an object schema and aborts the whole
/// turn.
///
/// Recovery, in order:
/// 1. Already a JSON object → leave it (the common, valid case).
/// 2. A JSON-encoded string (optionally fenced) that decodes to an object →
///    decode and use it.
/// 3. Otherwise a non-object whose tool schema declares **no** required fields →
///    coerce to `{}` (legacy-engine parity: the tool runs and produces its own
///    recoverable error).
/// 4. Otherwise (non-object + schema has required fields) → leave the arguments
///    untouched so the crate's `InvalidArgsPolicy::ReturnToolError` path reports
///    the original validation failure. Coercing to `{}` would discard the raw
///    malformed value without improving recovery.
pub(crate) struct ArgRecoveryMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's schema so we can tell whether coercing to `{}` is safe.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ArgRecoveryMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    fn schema_for(&self, name: &str) -> Option<ToolSchema> {
        schema_for_tool(&self.tool_sets, name)
    }
}

#[async_trait]
impl Middleware<()> for ArgRecoveryMiddleware {
    fn name(&self) -> &str {
        "arg_recovery"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // (1) Already valid object shape — nothing to do.
        if call.arguments.is_object() {
            return Ok(());
        }

        // (2) JSON-encoded-string arguments (optionally markdown-fenced): decode
        // and adopt the inner object.
        if let Some(raw) = call.arguments.as_str() {
            if let Some(obj) = recover_object_from_json_string(raw) {
                tracing::debug!(
                    tool = call.name.as_str(),
                    "[tinyagents::mw] arg_recovery: decoded JSON-encoded-string tool arguments to object"
                );
                call.arguments = obj;
                return Ok(());
            }
        }

        // (3) Non-object with a permissive schema (no required fields): coerce to
        // `{}` so the tool runs and produces its own recoverable error — engine
        // parity for tools that predate the schema gate.
        let has_required = self
            .schema_for(&call.name)
            .map(|schema| schema_has_required_fields(&schema.parameters))
            .unwrap_or(false);
        if !has_required {
            tracing::debug!(
                tool = call.name.as_str(),
                "[tinyagents::mw] arg_recovery: coercing non-object tool arguments to {{}} (schema declares no required fields)"
            );
            call.arguments = serde_json::json!({});
            return Ok(());
        }

        // (4) Non-object + schema has required fields: leave untouched. The
        // crate admission policy surfaces a descriptive, recoverable tool
        // result without executing the real tool.
        tracing::debug!(
            tool = call.name.as_str(),
            args_kind = json_value_kind(&call.arguments),
            "[tinyagents::mw] arg_recovery: leaving non-object tool arguments for crate invalid-args recovery"
        );
        Ok(())
    }
}

/// Resolves the harness [`ToolSchema`] for `name` across the runner's shared
/// tool sets.
///
/// Built via the same [`spec_to_schema`](super::convert::spec_to_schema)
/// conversion the runner uses for [`SharedToolAdapter::schema`], so the
/// `parameters` we validate against are byte-identical to the ones the crate's
/// fatal `validate_call` gate checks — otherwise our pre-validation could
/// disagree with the crate and either miss a fatal case or stub a call the crate
/// still rejects.
fn schema_for_tool(tool_sets: &[Arc<Vec<Box<dyn Tool>>>], name: &str) -> Option<ToolSchema> {
    tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .find(|tool| tool.name() == name)
        .map(|tool| super::convert::spec_to_schema(&tool.spec()))
}

/// Whether a tool's JSON-schema `parameters` declares any `required` field.
fn schema_has_required_fields(parameters: &serde_json::Value) -> bool {
    parameters
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|required| required.iter().any(serde_json::Value::is_string))
        .unwrap_or(false)
}

/// Attempts to recover a JSON **object** from a string-encoded arguments payload:
/// providers sometimes emit the whole arguments blob as a JSON string, optionally
/// wrapped in a ```json markdown fence. Returns `None` when the string does not
/// decode to a JSON object.
fn recover_object_from_json_string(raw: &str) -> Option<serde_json::Value> {
    let candidate = strip_code_fence(raw);
    serde_json::from_str::<serde_json::Value>(candidate)
        .ok()
        .filter(serde_json::Value::is_object)
}

/// Strips a surrounding markdown code fence (```` ```json … ``` ````) and its
/// optional language tag, returning the inner text. A string with no fence is
/// returned trimmed and unchanged.
fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Drop an optional language tag on the opening fence line (e.g. `json`).
    let body = match after_open.find('\n') {
        Some(newline)
            if after_open[..newline]
                .chars()
                .all(|c| c.is_ascii_alphanumeric()) =>
        {
            &after_open[newline + 1..]
        }
        _ => after_open,
    };
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

/// A short, human-readable kind label for a JSON value, for debug logging.
fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Agents are told to follow a **read-index → dedupe → write → update-index**
/// cycle around durable memory, but the contract was never enforced, so it was
/// followed inconsistently: writes landed without a dedupe read (duplicating
/// entries) and `update_memory_md` was skipped (so `MEMORY.md` drifted from the
/// store). This middleware observes the ordered sequence of *successful* memory
/// tool calls via [`MemoryProtocolTracker`] and, on each memory write, appends a
/// corrective note to the tool result so the model is nudged back onto the
/// protocol — the same "structured correction surfaced to the model" pattern the
/// unknown-tool recovery (#4118) uses. At run end it warns when a write was never
/// followed by an index update (the index is left stale).
///
/// Only *successful* ops advance the state machine — a failed `memory_store`
/// neither creates an entry nor obliges an index update. Non-memory tools are
/// ignored, so this is a no-op on turns that never touch memory.
pub struct MemoryProtocolMiddleware {
    tracker:
        std::sync::Mutex<crate::openhuman::agent::harness::memory_protocol::MemoryProtocolTracker>,
    /// call_id → classified op, captured in `before_tool` (the tool result carries
    /// no arguments, yet `update_memory_md` and `memory_tree` can only be
    /// classified from their `file` / `mode` argument). Correlated back by
    /// `result.call_id` in `after_tool`.
    pending_ops: std::sync::Mutex<
        std::collections::HashMap<
            String,
            crate::openhuman::agent::harness::memory_protocol::MemoryOp,
        >,
    >,
}

impl MemoryProtocolMiddleware {
    pub fn new() -> Self {
        Self {
            tracker: std::sync::Mutex::new(
                crate::openhuman::agent::harness::memory_protocol::MemoryProtocolTracker::new(),
            ),
            pending_ops: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for MemoryProtocolMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware<()> for MemoryProtocolMiddleware {
    fn name(&self) -> &str {
        "memory_protocol"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // Classify with the arguments in hand (the result won't carry them) and
        // stash the op keyed by call id. Only memory-relevant ops are stored, so
        // the map stays empty on turns that never touch memory.
        let op = crate::openhuman::agent::harness::memory_protocol::classify_memory_op(
            &call.name,
            &call.arguments,
        );
        if op != crate::openhuman::agent::harness::memory_protocol::MemoryOp::Other {
            if let Ok(mut ops) = self.pending_ops.lock() {
                ops.insert(call.id.clone(), op);
            }
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // Consume the op captured for this call (removing it so the map can't
        // grow unbounded). Absent → a non-memory tool: nothing to enforce.
        let op = self
            .pending_ops
            .lock()
            .ok()
            .and_then(|mut ops| ops.remove(&result.call_id));
        let Some(op) = op else {
            return Ok(());
        };
        // Only successful memory ops advance the protocol — a failed write did
        // not mutate memory and must not demand an index update.
        if result.error.is_some() {
            return Ok(());
        }
        let observation = {
            let mut tracker = match self.tracker.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            tracker.observe(op)
        };
        if let Some(note) = observation.guidance(&result.name) {
            tracing::debug!(
                tool = result.name.as_str(),
                missing_index_read = observation.missing_index_read,
                index_drift = observation.index_drift,
                "[tinyagents::mw] memory-protocol guidance appended to tool result"
            );
            if !result.content.is_empty() {
                result.content.push_str("\n\n");
            }
            result.content.push_str(&note);
        }
        Ok(())
    }

    async fn after_agent(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        _run: &mut AgentRun,
    ) -> TaResult<()> {
        let pending = self
            .tracker
            .lock()
            .map(|tracker| tracker.pending_index_update())
            .unwrap_or(false);
        if pending {
            tracing::warn!(
                "[tinyagents::mw] memory-protocol: run ended with a memory write that was never \
                 followed by update_memory_md — the MEMORY.md index is left stale"
            );
        }
        Ok(())
    }
}
