//! Host [`SecurityGate`] — OpenHuman's answer to "may this tool run?" and
//! "may this text enter the model context?".
//!
//! This is `docs/specs/plan-agents.md` Phase 4 for the most policy-sensitive
//! seam in the set. The tinyagents runtime holds *no* authority: it asks, and
//! obeys. Everything it would otherwise have to guess at lives on this side —
//! the autonomy tier, the security module's shell verdict, the per-channel tool policy, the
//! human-in-the-loop approval park, and the prompt-injection screen.
//!
//! # Domains adapted
//!
//! - [`crate::security::policy`] — [`SecurityPolicy`], its
//!   [`SecurityPolicy::check_command`] (the module's shell verdict) and
//!   [`AutonomyLevel`].
//! - [`crate::approval`] — the process-global [`ApprovalGate`],
//!   [`GateOutcome`], and the `summarize_action` / `redact_args` pair that keep
//!   raw arguments out of the approval card.
//! - [`crate::agent_tool_policy`] — a pre-built [`ToolPolicySession`]
//!   (channel permission boundary).
//! - [`crate::prompt_injection`] — [`enforce_prompt_input`] for
//!   [`SecurityGate::screen_input`].
//!
//! # Contract mismatches, resolved in favour of the existing guard
//!
//! 1. **"Answer, do not act" vs. the rate limiter.** OpenHuman's
//!    [`SecurityPolicy::enforce_tool_operation`] *records* an action against the
//!    sliding-window tracker as a side effect of authorizing one. The trait
//!    forbids acting, and the tools call it themselves anyway, so this adapter
//!    reads the non-mutating [`SecurityPolicy::is_rate_limited`] instead. A gate
//!    that called `enforce_tool_operation` would double-count every tool call
//!    and exhaust the hourly budget at twice the configured rate.
//! 2. **This gate owns approval, and must be the only thing that prompts.**
//!    `intercept_audited` hands back a `request_id` so the caller can log the
//!    *terminal* status once the tool resolves, but this trait returns before
//!    the tool runs and is never called again. The tempting shape — park here
//!    with the plain [`ApprovalGate::intercept`] and let
//!    `ApprovalSecurityMiddleware` record the terminal row — does not work: the
//!    middleware can only obtain an id by issuing its *own* `intercept_audited`,
//!    which raises a **second** approval card for a call the user already
//!    approved. A one-shot approval does not settle that second request, so the
//!    call would wait out a second TTL and then be denied.
//!
//!    So ownership is consolidated here. This adapter calls
//!    `intercept_audited`, stashes the id under the call id, and exposes
//!    [`OpenHumanSecurityGate::take_audit_request_id`] for the executor to drain
//!    and hand to [`ApprovalGate::record_execution`]. **A runner that installs
//!    this gate must not also compose `ApprovalSecurityMiddleware`'s approval
//!    step** — the two are alternatives, not layers. The session-allowlist
//!    shortcut yields no id and needs no terminal row.
//! 3. **No `Redacted` outcome is ever produced.** OpenHuman can *detect* PII
//!    ([`tinymemory_api::safety::scan`]) but exposes no verified
//!    public helper that rewrites free text into a redacted copy — the one that
//!    exists (`approval::redact::scrub_paths`) is private and shaped for JSON
//!    argument maps. Screening therefore only ever passes or blocks. See the
//!    `TODO(phase4)` in [`OpenHumanSecurityGate::screen_input`].
//! 4. **An absent approval gate allows, it does not deny.** When
//!    [`ApprovalGate::try_global`] is `None` (CLI, headless embed, tests) a
//!    `Prompt` decision cannot be resolved by a human. This adapter warns
//!    loudly and allows — byte-for-byte the behaviour of the
//!    `ApprovalSecurityMiddleware` it stands in for. Changing it to a denial
//!    here would be a *new* restriction invented by an adapter, not the
//!    honouring of an existing guard, and would break every non-desktop host.
//!    The deterministic `Block` path is unaffected: it never consults the gate.
//!
//!    **One exception, and it runs the other way.** A `RequireApproval` verdict
//!    from the *channel tool policy* denies when no gate is installed. That is
//!    not an invented restriction: `agent_tool_policy::engine` already files
//!    `RequireApproval` under `blocked_tool_names` alongside `Deny`, so the
//!    thing being honoured is a block, and the only thing that can lift it is a
//!    human actually approving. Allowing instead would turn the channel's
//!    strictest non-deny setting into its weakest on precisely the hosts with
//!    no human present. See [`ToolPolicyVerdict`].
//!
//! Nothing here widens a permission. Every branch that cannot establish
//! permission returns [`GateDecision::Deny`], including an unrecognised tool
//! name.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::host::security_gate::{GateDecision, ToolCallRequest};

use crate::agent::tinyagents::policy_denial::PolicyDenial;
use crate::security::approval::{
    redact_args, summarize_action, ApprovalGate, ApprovalTurnContext, GateOutcome,
};
use crate::security::policy::{
    CommandFlow, CommandQuery, CommandStage, GateDecision as PolicyGateDecision, SecurityPolicy,
};
use tinytools::{PermissionLevel, Tool};
use tinytools::{ToolPolicyAction, ToolPolicySession};

/// The tool name whose arguments carry a shell command string.
///
/// Command classification is not a generic capability: only `shell` has a
/// `command` argument for the security module to classify. Every other tool is
/// classified by its declared permission level and `external_effect_with_args`,
/// which is what the rest of the harness does too.
pub(super) const SHELL_TOOL: &str = "shell";

/// The channel tool-policy's verdict for one tool, as three explicit outcomes.
///
/// Deliberately not `Option<String>`. That shape can only say "denied" or
/// "nothing to say", so `RequireApproval` had to be encoded as the latter — and
/// silently became an allow for every tool that does not reach the approval
/// park by another route. Making the third outcome nameable is what stops that
/// recurring.
enum ToolPolicyVerdict {
    /// The channel permits this tool.
    Allow,
    /// The channel permits it only with human approval.
    RequireApproval,
    /// The channel forbids it; the payload is the rendered agent-facing reason.
    Deny(String),
}

/// OpenHuman's [`SecurityGate`].
///
/// Holds a boot-time [`SecurityPolicy`] snapshot, the session's optional
/// [`ToolPolicySession`], and the same `Arc`-shared tool sets the harness
/// registers — the last is how a call's declared permission level and
/// external-effect classification are recovered from a bare tool name.
pub struct OpenHumanSecurityGate {
    /// Fallback policy used when no process-global live policy is installed.
    ///
    /// Per-call resolution prefers
    /// [`crate::security::live_policy::effective`] so an autonomy
    /// change made mid-session is observed on the very next tool call — the
    /// same live-first / snapshot-fallback discipline `ApprovalGate` uses for
    /// `auto_approve`. The trait explicitly forbids the runtime caching a
    /// verdict for exactly this reason.
    policy: Arc<SecurityPolicy>,
    /// Channel permission boundary for this session, when one was built.
    ///
    /// `None` means the session was built without a tool-policy snapshot (the
    /// legacy unrestricted surface); the autonomy and approval checks below
    /// still apply.
    tool_policy: Option<Arc<ToolPolicySession>>,
    /// The tool sets the runner registered, used to resolve a call's `Tool` by
    /// name. An empty registry denies every call — see
    /// [`Self::resolve_tool`].
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    /// `pending_approvals` request ids from approvals this gate granted, keyed
    /// by the call they belong to and drained by
    /// [`Self::take_audit_request_id`].
    ///
    /// Exists so the audit row can be completed *without* a second
    /// `intercept_audited`. Issuing one to obtain an id is what would raise a
    /// second approval card for a call the user already approved once — see
    /// mismatch (2) in the module header.
    pending_audit: Mutex<HashMap<String, String>>,
    /// `false` for a host-only turn reading untrusted data
    /// ([`Self::with_input_screening`]): its input is data, not instructions.
    screens_input: bool,
    /// The owning turn's approval context (chat routing, copilot flag, flow
    /// run), read from the turn's explicit run context when the gate is built.
    approval: ApprovalTurnContext,
    /// The owning turn's dispatch context, read from the turn's run context
    /// when the gate is built; `None` falls back to the calling task's.
    dispatch: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
}

impl OpenHumanSecurityGate {
    /// Builds a gate over `policy` and the runner's shared `tool_sets`.
    ///
    /// `tool_sets` is not optional on purpose. Without it a tool name cannot be
    /// mapped to a permission level or an external-effect classification, and
    /// the only honest answer to "may this unknown thing run?" is no. Passing
    /// an empty vec therefore denies everything rather than silently degrading
    /// to allow-by-default.
    pub fn new(policy: Arc<SecurityPolicy>, tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self {
            policy,
            tool_policy: None,
            tool_sets,
            pending_audit: Mutex::new(HashMap::new()),
            screens_input: true,
            approval: ApprovalTurnContext::default(),
            dispatch: None,
        }
    }

    /// Binds the owning turn's dispatch context.
    pub fn with_dispatch(mut self, dispatch: DispatchCtx) -> Self {
        self.dispatch = dispatch.downgrade();
        self
    }

    /// Binds the owning turn's approval context, so a parked approval is routed
    /// to the turn's chat surface and stamped with its flow run.
    pub fn with_approval(mut self, approval: ApprovalTurnContext) -> Self {
        self.approval = approval;
        self
    }

    /// Whether this gate screens the turn's input. A host-only turn reading
    /// untrusted data (`OpenHumanRunContext::untrusted_input_turn`) passes
    /// `false`: its input is data, not instructions.
    pub fn with_input_screening(mut self, screens_input: bool) -> Self {
        self.screens_input = screens_input;
        self
    }

    /// Attaches the session's channel permission boundary.
    pub fn with_tool_policy(mut self, session: Arc<ToolPolicySession>) -> Self {
        self.tool_policy = Some(session);
        self
    }

    /// Removes and returns the `pending_approvals` request id this gate
    /// recorded for `call_id`, if it granted an approval for that call.
    ///
    /// The executor calls this once the tool resolves and passes the id to
    /// [`ApprovalGate::record_execution`], completing the before-and-after
    /// audit row (#2135). Draining is deliberate: an id is valid for exactly
    /// one terminal record, and leaving it behind would grow the map for the
    /// life of the session.
    ///
    /// `None` means there is nothing to record — the call was auto-approved via
    /// the session allowlist, was never prompted, or carried no `call_id`.
    pub fn take_audit_request_id(&self, call_id: &str) -> Option<String> {
        self.pending_audit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(call_id)
    }

    /// The policy to answer this call against: the agent's or the live one when
    /// installed, else the constructor snapshot.
    fn effective_policy(&self) -> Arc<SecurityPolicy> {
        crate::security::live_policy::effective().unwrap_or_else(|| self.policy.clone())
    }

    /// Finds the registered [`Tool`] named `name`, if any.
    fn resolve_tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// The channel-permission verdict for `tool_name`, or `None` when this
    /// session carries no tool policy.
    ///
    /// `RequireApproval` returns [`ToolPolicyVerdict::RequireApproval`], which
    /// the caller routes to the approval park.
    ///
    /// It previously returned "no verdict" on the theory that the call would
    /// fall through to the park anyway. **It does not.** The park is reached
    /// only from the `shell` branch and the external-effect branch, so any
    /// other tool with a `RequireApproval` channel decision reached the end of
    /// `authorize_tool` and was allowed with nobody asked. That is the opposite
    /// of the host's own semantics: `agent_tool_policy::engine` puts
    /// `RequireApproval` in `blocked_tool_names` alongside `Deny`, and the
    /// runtime-side middleware renders it as `PolicyDenial::ApprovalRequired`.
    fn tool_policy_verdict(&self, tool_name: &str) -> ToolPolicyVerdict {
        let Some(session) = self.tool_policy.as_ref() else {
            return ToolPolicyVerdict::Allow;
        };
        let decision = session.decision_for(tool_name);
        match decision.action {
            ToolPolicyAction::Allow => ToolPolicyVerdict::Allow,
            ToolPolicyAction::RequireApproval => ToolPolicyVerdict::RequireApproval,
            // `HideFromPrompt` is a denial too: a tool the model was never shown
            // and named anyway is the case this boundary exists for.
            ToolPolicyAction::Deny | ToolPolicyAction::HideFromPrompt => ToolPolicyVerdict::Deny(
                PolicyDenial::SessionForbidden {
                    tool: tool_name,
                    required: decision.required_permission,
                    allowed: decision.allowed_permission,
                    channel: &session.profile.channel,
                }
                .render(),
            ),
        }
    }

    /// Asks the TinySecurity module about a `shell` call and maps its verdict
    /// onto this trait's.
    ///
    /// The module classifies the command, applies the autonomy tier and the
    /// hidden-execution and protected-path defences, and raises (never lowers)
    /// the class with the model's self-declared `category`. A module that cannot
    /// answer refuses the command. Asking here does not spend the hourly budget;
    /// the run itself does.
    async fn shell_decision(
        policy: &SecurityPolicy,
        args: &serde_json::Value,
    ) -> std::result::Result<PolicyGateDecision, String> {
        let command = args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                format!(
                    "{} Shell command is missing.",
                    tinytools::status::POLICY_BLOCKED_MARKER
                )
            })?;
        let declared = args
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(SecurityPolicy::parse_declared_class);
        let verdict = policy
            .check_command(CommandQuery {
                command,
                declared,
                flow: CommandFlow::Harness,
                stage: CommandStage::Authorize,
            })
            .await
            .map_err(|refusal| refusal.to_string())?;
        Ok(verdict.gate)
    }

    /// The allow-shaped answer for a call that reached the end of the stages.
    ///
    /// Reports `Prompted { approved: true }` rather than a bare `Allow` when a
    /// human already approved this call at the channel stage, so the runtime is
    /// told a person was consulted instead of being shown a decision that looks
    /// automatic.
    fn settled(&self, channel_approved: bool) -> GateDecision {
        if channel_approved {
            GateDecision::Prompted { approved: true }
        } else {
            GateDecision::Allow
        }
    }

    /// Parks for approval unless this call was already approved.
    ///
    /// The channel policy and a later stage can both want a prompt for the same
    /// call. Asking twice would show the user two cards for one action and, with
    /// a one-shot approval, let the second request expire — so an approval
    /// already granted stands.
    async fn park_once(&self, call: &ToolCallRequest, already_approved: bool) -> GateDecision {
        if already_approved {
            tracing::debug!(
                target: "tinyagents",
                tool = %call.tool_name,
                "[tinyagents::host::security] already approved at the channel stage; not \
                 prompting again"
            );
            return GateDecision::Prompted { approved: true };
        }
        self.park_for_approval(call).await
    }

    /// Parks the turn on the human approval flow and reports how it settled.
    ///
    /// An approval returns [`GateDecision::Prompted`]; a refusal — a Deny, the
    /// TTL timeout, or a dropped channel, all of which `ApprovalGate` renders as
    /// a `Deny` — returns [`GateDecision::Deny`] via [`decision_for_outcome`].
    /// A `Prompted { approved: false }` carries no reason, so the harness fell
    /// back to a bare "tool call was not approved": the model then asked the
    /// user to approve, or reached the same result through `shell` instead.
    /// The Deny text gives it the parity `ApprovalSecurityMiddleware` already
    /// had, while still never hinting that a human was (or was not) at the
    /// keyboard.
    async fn park_for_approval(&self, call: &ToolCallRequest) -> GateDecision {
        let Some(gate) = ApprovalGate::try_global() else {
            // Parity with `ApprovalSecurityMiddleware`: no gate installed means
            // no interactive approval exists in this host (CLI / headless /
            // tests). See mismatch (4) in the module header.
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                agent = %call.agent_id,
                "[tinyagents::host::security] approval gate unavailable; allowing a call that \
                 would otherwise prompt"
            );
            return GateDecision::Allow;
        };
        self.park_for_approval_on(&gate, call).await
    }

    /// [`Self::park_for_approval`] against an explicit gate, with this turn's
    /// approval context.
    pub(crate) async fn park_for_approval_on(
        &self,
        gate: &ApprovalGate,
        call: &ToolCallRequest,
    ) -> GateDecision {
        let summary = summarize_action(&call.tool_name, &call.arguments);
        let redacted = redact_args(&call.arguments);
        tracing::debug!(
            target: "tinyagents",
            tool = %call.tool_name,
            agent = %call.agent_id,
            call_id = ?call.call_id.as_ref().map(|c| c.as_str()),
            "[tinyagents::host::security] parking tool call on the approval gate"
        );
        // `intercept_audited`, not `intercept`: the returned id is what lets the
        // executor close the audit row later *without* raising a second
        // approval card. See mismatch (2).
        let (outcome, request_id) = gate
            .intercept_audited(&self.approval, &call.tool_name, &summary, redacted)
            .await;
        if matches!(outcome, GateOutcome::Allow) {
            // Only an approval that persisted a row yields an id; the
            // session-allowlist shortcut returns `None` and has nothing to
            // record. A call with no `call_id` cannot be correlated back, so
            // the id is dropped rather than stored under a key no executor can
            // ask for.
            if let (Some(request_id), Some(call_id)) = (request_id, call.call_id.as_ref()) {
                self.pending_audit
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(call_id.as_str().to_string(), request_id);
            }
        }
        decision_for_outcome(&call.tool_name, outcome)
    }
    async fn authorize_tool_inner(&self, call: &ToolCallRequest) -> TaResult<GateDecision> {
        // A nested call (made by a running tool, not the model) can never be
        // parked for a human, and the stages below prompt. Fail closed rather
        // than prompt or guess which stages would have: nested calls are off
        // (`RunLimits::max_nested_depth = 0`) and stay refused here until the
        // host decides how to authorize them.
        if call.is_nested() {
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                "[tinyagents::host::security] nested tool call refused (fail closed)"
            );
            return Ok(GateDecision::deny(
                PolicyDenial::PolicyDenied {
                    tool: &call.tool_name,
                    policy: "nested tool calls",
                    reason: "Tools may not call other tools in this session.",
                }
                .render(),
            ));
        }
        let policy = self.effective_policy();
        #[cfg(feature = "modules")]
        let desktop_approval_disabled = match self.resolve_tool(&call.tool_name) {
            Some(tool) => {
                crate::desktop::control::approvals_disabled_for(
                    &DispatchCtx::from_weak(self.dispatch.as_ref()),
                    tool,
                )
                .await
            }
            None => false,
        };
        #[cfg(not(feature = "modules"))]
        let desktop_approval_disabled = false;
        tracing::debug!(
            target: "tinyagents",
            tool = %call.tool_name,
            agent = %call.agent_id,
            autonomy = ?policy.autonomy,
            "[tinyagents::host::security] authorizing tool call"
        );

        // Set when the channel policy demanded approval and the human granted
        // it. Carried forward so a later stage that would prompt for the *same*
        // call does not raise a second card for an answer already given.
        let mut channel_approved = false;

        // 1. Channel permission boundary.
        match self.tool_policy_verdict(&call.tool_name) {
            ToolPolicyVerdict::Allow => {}
            ToolPolicyVerdict::Deny(reason) => {
                tracing::warn!(
                    target: "tinyagents",
                    tool = %call.tool_name,
                    "[tinyagents::host::security] denied by the session tool policy"
                );
                return Ok(GateDecision::Deny { reason });
            }
            ToolPolicyVerdict::RequireApproval if desktop_approval_disabled => {
                tracing::debug!(tool = %call.tool_name,
                    "[desktop] channel approval park skipped by desktop setting");
            }
            ToolPolicyVerdict::RequireApproval => {
                // The channel says a human must approve. Unlike the shell and
                // external-effect paths below, an absent approval gate must
                // **deny** here rather than allow: `agent_tool_policy` classes
                // `RequireApproval` as blocked, so the only thing that can
                // unblock it is an actual approval. Falling back to allow on a
                // headless host would turn the channel's strictest non-deny
                // setting into its weakest.
                if ApprovalGate::try_global().is_none() {
                    tracing::warn!(
                        target: "tinyagents",
                        tool = %call.tool_name,
                        "[tinyagents::host::security] channel policy requires approval but no \
                         approval gate is installed; denying"
                    );
                    return Ok(GateDecision::deny(
                        PolicyDenial::ApprovalRequired {
                            tool: &call.tool_name,
                            policy: "channel tool policy",
                            reason: "This tool requires human approval, and no approval flow is \
                                     available in this session.",
                        }
                        .render(),
                    ));
                }
                // Approval here settles the **channel** restriction only. It is
                // not a general authorization: returning now would skip tool
                // resolution and the autonomy tier, so a `shell` command the
                // tier blocks, or any acting tool under `readonly`, would run
                // purely because the channel asked for a prompt. Each stage may
                // only narrow the answer, so record the human's answer and keep
                // going.
                match self.park_for_approval(call).await {
                    GateDecision::Prompted { approved: true } => {
                        channel_approved = true;
                    }
                    // A refusal (or an expired TTL) is terminal — no later
                    // stage can widen it back to an allow.
                    denial => return Ok(denial),
                }
            }
        }

        // 2. Resolve the tool. Fail closed on an unknown name: without the
        //    registered `Tool` there is no permission level and no
        //    external-effect classification to reason about.
        let Some(tool) = self.resolve_tool(&call.tool_name) else {
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                "[tinyagents::host::security] denied: tool is not registered in this session"
            );
            return Ok(GateDecision::deny(format!(
                "Tool '{}' is not available in this session, so it cannot be authorized. \
                 Use a registered tool or report that this cannot be done here.",
                call.tool_name
            )));
        };

        // A scheduled prompt is data, not an open-ended approval for later
        // actions. Keep cron agent turns within a read-only tool capability.
        // This check precedes the shell shortcut and all approval settings,
        // including installations where the approval gate is disabled.
        if matches!(
            crate::agent::turn_origin::current(),
            Some(
                crate::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                    source: crate::agent::turn_origin::TrustedAutomationSource::Cron,
                    ..
                }
            )
        ) && (call.tool_name == SHELL_TOOL
            || tool.permission_level_with_args(&call.arguments) > PermissionLevel::ReadOnly
            || tool.external_effect_with_args(&call.arguments))
        {
            return Ok(GateDecision::deny(format!(
                "{} Scheduled agent turns may use read-only tools only; '{}' needs a live user action.",
                tinytools::status::POLICY_DENIED_MARKER,
                call.tool_name
            )));
        }

        // 3. `shell` is the one tool whose arguments carry a classifiable
        //    command, and `gate_decision` already encodes the autonomy tier —
        //    at a finer grain than the tool's coarse `Execute` permission
        //    level. Running stage 4 first would refuse `ls` in read-only mode,
        //    which OpenHuman's own `ShellTool` permits. So shell answers here
        //    and returns.
        if call.tool_name == SHELL_TOOL {
            match Self::shell_decision(&policy, &call.arguments).await {
                Err(raw_reason) => {
                    tracing::warn!(
                        target: "tinyagents",
                        tool = %call.tool_name,
                        "[tinyagents::host::security] shell command blocked by security policy"
                    );
                    return Ok(GateDecision::deny(
                        PolicyDenial::SecurityPolicyBlocked {
                            tool: &call.tool_name,
                            raw_reason: &raw_reason,
                        }
                        .render(),
                    ));
                }
                Ok(PolicyGateDecision::Block) => {
                    // The module refuses a blocked command with a reason, so
                    // this is not expected; refuse it all the same.
                    let raw = format!(
                        "{} Security policy: this command's category is not permitted in the \
                         current access tier.",
                        tinytools::status::POLICY_BLOCKED_MARKER
                    );
                    return Ok(GateDecision::deny(
                        PolicyDenial::SecurityPolicyBlocked {
                            tool: &call.tool_name,
                            raw_reason: &raw,
                        }
                        .render(),
                    ));
                }
                Ok(PolicyGateDecision::Prompt) => {
                    return Ok(self.park_once(call, channel_approved).await);
                }
                Ok(PolicyGateDecision::Allow) => {
                    return Ok(self.settled(channel_approved));
                }
            }
        }

        let required = tool.permission_level_with_args(&call.arguments);
        let acts = required > PermissionLevel::ReadOnly;

        // 4. Autonomy tier + budget. Read-only mode refuses every acting tool
        //    outright; no in-tier prompt can authorize it.
        if acts && !policy.can_act() {
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                %required,
                "[tinyagents::host::security] denied: read-only autonomy tier"
            );
            let raw = format!(
                "{} Security policy: read-only mode, cannot perform '{}'.",
                tinytools::status::POLICY_BLOCKED_MARKER,
                call.tool_name
            );
            return Ok(GateDecision::deny(
                PolicyDenial::SecurityPolicyBlocked {
                    tool: &call.tool_name,
                    raw_reason: &raw,
                }
                .render(),
            ));
        }
        // `is_rate_limited` is the read-only twin of `record_action`; the tools
        // still do the recording themselves (mismatch (1)).
        if acts && policy.is_rate_limited() {
            tracing::warn!(
                target: "tinyagents",
                tool = %call.tool_name,
                max_per_hour = policy.max_actions_per_hour,
                "[tinyagents::host::security] denied: hourly action budget exhausted"
            );
            return Ok(GateDecision::deny(format!(
                "Rate limit exceeded: action budget exhausted ({} actions/hour). Wait for the \
                 rolling one-hour window to refill, or raise the limit in Settings -> Advanced \
                 -> Agent autonomy.",
                policy.max_actions_per_hour
            )));
        }

        // 5. Everything else: the tool's own external-effect classification
        //    decides whether a human is asked.
        if tool.external_effect_with_args(&call.arguments) && !desktop_approval_disabled {
            return Ok(self.park_once(call, channel_approved).await);
        }

        tracing::debug!(
            target: "tinyagents",
            tool = %call.tool_name,
            "[tinyagents::host::security] allowed"
        );
        Ok(self.settled(channel_approved))
    }
}

#[path = "security_gate_approval.rs"]
mod approval;
use approval::decision_for_outcome;

#[path = "security_gate_impl.rs"]
mod security_gate_impl;

#[cfg(test)]
#[path = "security_gate_tests.rs"]
mod tests;
