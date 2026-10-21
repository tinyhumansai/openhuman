//! [`ToolPolicyMiddleware`]: enforce the builder-configured `ToolCallPolicy` and
//! the session's channel-permission ceiling at the tool boundary, and render
//! session-scoped `use_skill` listings.

use std::sync::Arc;

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{
    DenialRenderer, MiddlewareToolOutcome, PolicyDecision, ToolCallPolicy, ToolHandler,
    ToolMiddleware, ToolPolicyGateMiddleware,
};
use tinyagents_harness_tools::skills::PackRegistryHandle;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::policy_denial::PolicyDenial;
use tinytools::Tool;

/// `wrap_tool`: enforce the agent's builder-configured `ToolCallPolicy` at the tool
/// boundary (issue #4249). The in-house engine ran this check in
/// `agent_tool_exec` (`ctx.tool_policy.check(...)`); the tinyagents path bypassed
/// it, so a `.tool_policy()` deny/require-approval silently no-opped and the tool
/// executed anyway — a security regression. This middleware restores it: a
/// blocking decision short-circuits with a model-consumable result carrying the
/// same `"Tool '<name>' <denied|requires approval> by policy '<policy>': <reason>"`
/// wording the engine produced.
pub(crate) struct ToolPolicyMiddleware {
    /// TinyAgents owns evaluation and denial short-circuiting.
    gate: ToolPolicyGateMiddleware<OpenHumanRunContext>,
    /// OpenHuman's concrete channel ceiling and disclosure snapshot.
    #[cfg(test)]
    host: Arc<ToolPolicyHostState>,
}

struct ToolPolicyHostState {
    session: tinytools::ToolPolicySession,
    /// Shared tool sets (same `Arc`s the runner registers) so a call's OpenHuman
    /// `Tool` can be resolved for its generated-tool runtime context and its
    /// per-call permission level.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    channel: String,
}

/// The session-less allow-all gate for sessions with no builder policy.
#[derive(Debug, Default)]
pub(crate) struct AllowAllToolPolicy;

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for AllowAllToolPolicy {
    fn name(&self) -> &str {
        "allow_all"
    }

    async fn check(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
    ) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

impl ToolPolicyMiddleware {
    pub(crate) fn new(
        policy: Arc<dyn ToolCallPolicy<OpenHumanRunContext>>,
        session: tinytools::ToolPolicySession,
        tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
        channel: String,
    ) -> Self {
        let host = Arc::new(ToolPolicyHostState {
            session,
            tool_sets,
            channel,
        });
        let desktop_policy = Arc::new(DesktopApprovalPolicy {
            policy,
            tool_sets: host.tool_sets.clone(),
        });
        let session_ceiling = Arc::new(SessionCeilingPolicy { host: host.clone() });
        let denial: DenialRenderer = Arc::new(|call, policy, decision| {
            let (action, reason) = match decision {
                PolicyDecision::RequireApproval { reason } => {
                    ("requires approval", reason.as_str())
                }
                PolicyDecision::Deny { reason } => ("denied", reason.as_str()),
                PolicyDecision::Allow => ("allowed", ""),
            };
            tinytools::registry::denials::record(call.name.as_str(), policy, action, reason);
            if policy == "openhuman_session_ceiling" {
                return reason.to_owned();
            }
            match decision {
                PolicyDecision::RequireApproval { reason } => PolicyDenial::ApprovalRequired {
                    tool: &call.name,
                    policy,
                    reason,
                }
                .render(),
                PolicyDecision::Deny { reason } => PolicyDenial::PolicyDenied {
                    tool: &call.name,
                    policy,
                    reason,
                }
                .render(),
                PolicyDecision::Allow => String::new(),
            }
        });
        let post_host = host.clone();
        let gate = ToolPolicyGateMiddleware::new(desktop_policy)
            .with_session_ceiling(session_ceiling)
            .with_denial_renderer(denial)
            .with_post_gate(Arc::new(move |_ctx, call| {
                post_host.render_skill_for_session(call)
            }));
        Self {
            gate,
            #[cfg(test)]
            host,
        }
    }

    // Keep focused host-adapter tests on the concrete disclosure callbacks
    // after execution gating moves to TinyAgents. Production calls reach these
    // callbacks through `with_post_gate` and the session-ceiling policy above.
    #[cfg(test)]
    pub(crate) fn render_skill_for_session(&self, call: &TaToolCall) -> Option<TaToolResult> {
        self.host.render_skill_for_session(call)
    }

    #[cfg(test)]
    pub(crate) fn channel_permission_block(
        &self,
        call: &TaToolCall,
        desktop_approval_disabled: bool,
    ) -> Option<String> {
        self.host
            .channel_permission_block(call, desktop_approval_disabled)
    }

    #[cfg(test)]
    pub(crate) fn route_for_pack(
        &self,
        pack: &tinyagents_harness_tools::skills::ToolPack,
    ) -> String {
        self.host.route_for_pack(pack)
    }

    #[cfg(test)]
    pub(crate) fn no_such_pack_tool<'a>(
        &self,
        pack: &'static tinyagents_harness_tools::skills::ToolPack,
        tool: &'a str,
    ) -> tinyagents_harness_tools::skills::NoSuchPackTool<'a> {
        self.host.no_such_pack_tool(pack, tool)
    }
}

impl ToolPolicyHostState {
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
            let Some(target) = crate::tools::host_extensions::delegation_target(tool.as_ref())
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

    /// The answer to a `use_skill` call naming a tool `pack` does not contain:
    /// the tools in it this session can call, or the pack's route when none.
    pub(crate) fn no_such_pack_tool<'a>(
        &self,
        pack: &'static tinyagents_harness_tools::skills::ToolPack,
        tool: &'a str,
    ) -> tinyagents_harness_tools::skills::NoSuchPackTool<'a> {
        let callable = pack
            .tools
            .iter()
            .copied()
            .filter(|name| {
                self.resolve_tool(name).is_some()
                    && !self.session.decision_for(name).blocks_execution()
            })
            .collect();
        tinyagents_harness_tools::skills::NoSuchPackTool {
            not_found_marker: tinytools::status::NOT_FOUND_MARKER,
            skill: pack.id,
            tool,
            callable,
            route: self.route_for_pack(pack),
        }
    }

    /// The route sentence for a pack, resolved against THIS session.
    pub(crate) fn route_for_pack(
        &self,
        pack: &tinyagents_harness_tools::skills::ToolPack,
    ) -> String {
        tinyagents_harness_tools::skills::route_sentence(
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
    pub(crate) fn render_skill_for_session(&self, call: &TaToolCall) -> Option<TaToolResult> {
        if tinyagents_harness_tools::skills::named_tool(&call.arguments).is_some() {
            return None;
        }
        let skill = call
            .arguments
            .get("skill")
            .and_then(serde_json::Value::as_str)?;
        self.resolve_tool(&call.name)?;
        let durable = self.tool_sets.first()?;
        let handle = PackRegistryHandle::default();
        handle.bind(Arc::downgrade(durable));
        if let Some(synthesized) = self.tool_sets.get(1) {
            handle.bind_synthesized(Arc::downgrade(synthesized));
        }
        let is_callable = |name: &str| !self.session.decision_for(name).blocks_execution();
        let route = crate::tools::toolpacks::pack(skill)
            .map(|pack| self.route_for_pack(pack))
            .unwrap_or_default();
        let rendered = tinyagents_harness_tools::skills::render_pack_filtered(
            &crate::tools::toolpacks::CATALOG,
            skill,
            &handle,
            // The same predicate the gate applies to `use_skill`'s inner tool.
            // Two sources of truth for "can this session call it" is the bug.
            &is_callable,
            &route,
        );
        Some(match rendered {
            Ok(text) => TaToolResult::success(text),
            Err(message) => TaToolResult::error(message),
        })
    }

    /// The channel-permission gate the engine ran before the builder policy: a
    /// session-level deny, then a per-call permission-level ceiling check. Returns
    /// the blocking message when the call must not execute.
    pub(crate) fn channel_permission_block(
        &self,
        call: &TaToolCall,
        desktop_approval_disabled: bool,
    ) -> Option<String> {
        let decision = self.session.decision_for(&call.name);
        let approval_only = desktop_approval_disabled
            && matches!(
                decision.action,
                tinytools::ToolPolicyAction::RequireApproval
            );
        if decision.is_denied() && !approval_only {
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
        // For `use_skill`, validate the resolved inner tool against the session
        // allowlist. Role-hidden packed tools are not checked by the outer
        // policy name; without this check `use_skill` would bypass the
        // session's effective allowlist for any packed tool.
        //
        // This runs BEFORE the argument-level permission check below, and the
        // order is load-bearing. That check asks
        // `UseSkillTool::permission_level_with_args`, which reports the
        // pack-wide CEILING for an inner name it cannot resolve — so on a
        // channel sitting under that ceiling, an invented name came back as
        // `PermissionTooLow`: a permission denial for a tool that does not
        // exist. Existence is not a policy question (#6302).
        if call.name == "use_skill" {
            if let Some(inner_tool) = call
                .arguments
                .get("tool")
                .and_then(serde_json::Value::as_str)
            {
                // Existence before permission. An invented name (`install_skill`
                // in `skills`) is not a policy problem, and answering it with a
                // denial reads as "installs are forbidden" (#6302). A tool the
                // named skill does not contain gets that answer, with what this
                // session can call in the skill instead — and so does a pack
                // member this core never registered (feature gate, or an
                // embedder's `ToolGroups::Off`), which is equally absent no
                // matter what the static pack table says.
                if let Some(pack) = call
                    .arguments
                    .get("skill")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::tools::toolpacks::pack)
                {
                    if !pack.owns(inner_tool) || self.resolve_tool(inner_tool).is_none() {
                        return Some(self.no_such_pack_tool(pack, inner_tool).render());
                    }
                }
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
                    let hint = crate::tools::toolpacks::pack_for_tool(inner_tool)
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

        // Per-call permission ceiling, last: an unregistered tool has no level
        // to compare, and for `use_skill` the level comes from the INNER tool
        // (`permission_level_with_args`), which the block above has already
        // proved to exist and to be reachable in this session.
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
        None
    }
}

struct SessionCeilingPolicy {
    host: Arc<ToolPolicyHostState>,
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for SessionCeilingPolicy {
    fn name(&self) -> &str {
        "openhuman_session_ceiling"
    }

    async fn check(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> PolicyDecision {
        #[cfg(feature = "modules")]
        let approvals_disabled = if let Some(tool) = self.host.resolve_tool(&call.name) {
            crate::desktop::control::approvals_disabled_for(&ctx.data.dispatch_ctx(), tool.as_ref())
                .await
        } else {
            false
        };
        #[cfg(not(feature = "modules"))]
        let approvals_disabled = false;

        match self.host.channel_permission_block(call, approvals_disabled) {
            Some(reason) => PolicyDecision::deny(reason),
            None => PolicyDecision::Allow,
        }
    }
}

struct DesktopApprovalPolicy {
    policy: Arc<dyn ToolCallPolicy<OpenHumanRunContext>>,
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for DesktopApprovalPolicy {
    fn name(&self) -> &str {
        self.policy.name()
    }

    async fn check(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> PolicyDecision {
        let decision = self.policy.check(ctx, call).await;
        if !matches!(decision, PolicyDecision::RequireApproval { .. }) {
            return decision;
        }
        #[cfg(feature = "modules")]
        let approvals_disabled = if let Some(tool) = self
            .tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|tool| tool.name() == call.name)
        {
            crate::desktop::control::approvals_disabled_for(&ctx.data.dispatch_ctx(), tool.as_ref())
                .await
        } else {
            false
        };
        #[cfg(not(feature = "modules"))]
        let approvals_disabled = false;
        if approvals_disabled {
            PolicyDecision::Allow
        } else {
            decision
        }
    }
}

#[async_trait]
impl ToolMiddleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for ToolPolicyMiddleware
{
    fn name(&self) -> &str {
        "tool_policy"
    }

    async fn wrap_tool(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> TaResult<MiddlewareToolOutcome> {
        self.gate.wrap_tool(ctx, state, call, next).await
    }
}
