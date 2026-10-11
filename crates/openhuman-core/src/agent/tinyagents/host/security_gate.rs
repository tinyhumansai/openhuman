//! Host [`SecurityGate`] — OpenHuman's answer to "may this tool run?" and
//! "may this text enter the model context?".
//!
//! This is `docs/specs/plan-agents.md` Phase 4 for the most policy-sensitive
//! seam in the set. The tinyagents runtime holds *no* authority: it asks, and
//! obeys. Everything it would otherwise have to guess at lives on this side —
//! the autonomy tier, policy over TinyBox's command facts, the per-channel tool policy, the
//! human-in-the-loop approval park, and the prompt-injection screen.
//!
//! # Domains adapted
//!
//! - [`crate::security::policy`] — [`SecurityPolicy`], its
//!   [`SecurityPolicy::check_gated_command_analysis`] / [`SecurityPolicy::gate_decision`]
//!   decisions, and [`AutonomyLevel`].
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Deserialize;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::host::security_gate::{
    ContentOrigin, GateDecision, ScreenOutcome, SecurityGate, ToolCallRequest,
};

use crate::agent::tinyagents::policy_denial::PolicyDenial;
use crate::security::approval::{redact_args, summarize_action, ApprovalGate, GateOutcome};
use crate::security::policy::{CommandClass, GateDecision as PolicyGateDecision, SecurityPolicy};
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::tools::agent_policy::{ToolPolicyAction, ToolPolicySession};
use tinytools::{PermissionLevel, Tool};

/// The tool name whose arguments carry a shell command string.
///
/// Command classification is not a generic capability: only `shell` has a
/// `command` argument for TinyBox to analyze. Every other tool is classified by its declared permission level and
/// `external_effect_with_args`, which is what the rest of the harness does too.
const SHELL_TOOL: &str = "shell";

#[derive(Deserialize)]
struct PreparedShellAuthorization {
    tool_name: String,
    request_arguments: serde_json::Value,
    effective_arguments: serde_json::Value,
    tool_facts: serde_json::Value,
}

fn shell_analysis_for_call(
    call: &ToolCallRequest,
    context: Option<&serde_json::Value>,
) -> Option<(serde_json::Value, serde_json::Value)> {
    let prepared: PreparedShellAuthorization = serde_json::from_value(context?.clone()).ok()?;
    if prepared.tool_name != SHELL_TOOL || prepared.request_arguments != call.arguments {
        return None;
    }
    Some((
        prepared.effective_arguments,
        prepared.tool_facts.get("analysis")?.clone(),
    ))
}

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
    /// ([`with_untrusted_input_turn`]): its input is data, not instructions.
    screens_input: bool,
}

tokio::task_local! {
    static UNTRUSTED_INPUT_TURN: ();
}

/// Run `fut` as a turn whose input is untrusted data: gates built for it do
/// not screen input. Scoped only by `agent_chat_reply_for`, and only after the
/// session accepted untrusted input, which only a host-only session does.
pub(crate) async fn with_untrusted_input_turn<F: std::future::Future>(fut: F) -> F::Output {
    UNTRUSTED_INPUT_TURN.scope((), Box::pin(fut)).await
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
            // Read here, on the turn's own task: a screen may run elsewhere.
            screens_input: UNTRUSTED_INPUT_TURN.try_with(|_| ()).is_err(),
        }
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

    /// Classifies a `shell` call and maps OpenHuman's three-way policy verdict
    /// onto this trait's.
    ///
    /// Applies host policy to TinyBox's generic command facts, then raises (never
    /// lowers) the result with the model's self-declared `category`.
    #[cfg(feature = "modules")]
    fn shell_decision(
        policy: &SecurityPolicy,
        args: &serde_json::Value,
        analysis: &serde_json::Value,
    ) -> std::result::Result<PolicyGateDecision, String> {
        let analysis: crate::modules::tinybox::contract::CommandAnalysis =
            serde_json::from_value(analysis.clone())
                .map_err(|_| "TinyBox command analysis is invalid".to_owned())?;
        let mut class: CommandClass = policy.check_gated_command_analysis(&analysis)?;
        if let Some(declared) = args
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(SecurityPolicy::parse_declared_class)
        {
            class = class.max(declared);
        }
        Ok(policy.gate_decision(class))
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
            .intercept_audited(&call.tool_name, &summary, redacted)
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
    async fn authorize_tool_with_facts(
        &self,
        call: &ToolCallRequest,
        authorization_context: Option<&serde_json::Value>,
    ) -> TaResult<GateDecision> {
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
            Some(tool) => crate::desktop::control::approvals_disabled_for(tool).await,
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
                crate::security::POLICY_DENIED_MARKER,
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
            #[cfg(feature = "modules")]
            {
                let Some((effective_arguments, analysis)) =
                    shell_analysis_for_call(call, authorization_context)
                else {
                    return Ok(GateDecision::deny(format!(
                        "{} Shell authorization facts are unavailable or do not match this call.",
                        crate::security::POLICY_BLOCKED_MARKER
                    )));
                };
                match Self::shell_decision(&policy, &effective_arguments, &analysis) {
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
                        // `check_gated_command_analysis` already rejects the read-only Block
                        // case; this arm catches a Block produced by the model's
                        // escalate-only `category` hint.
                        let raw = format!(
                            "{} Security policy: this command's category is not permitted in the \
                         current access tier.",
                            crate::security::POLICY_BLOCKED_MARKER
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
            #[cfg(not(feature = "modules"))]
            {
                let _ = authorization_context;
                return Ok(GateDecision::deny(format!(
                    "{} Shell command analysis is unavailable in this build.",
                    crate::security::POLICY_BLOCKED_MARKER
                )));
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
                crate::security::POLICY_BLOCKED_MARKER,
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

#[async_trait]
impl SecurityGate for OpenHumanSecurityGate {
    /// Answers in five stages, each of which can only narrow the answer:
    ///
    /// 1. the channel tool policy (`agent_tool_policy`),
    /// 2. tool resolution — an unregistered name is denied,
    /// 3. `shell` command classification, which supersedes stage 4 for that one
    ///    tool because it is strictly finer-grained,
    /// 4. the autonomy tier for acting tools (`can_act`) plus the non-mutating
    ///    rate-limit read, and
    /// 5. the approval park for anything the tier prompts on.
    ///
    /// **A human approval never short-circuits the remaining stages.** Approving
    /// at stage 1 settles the *channel* restriction and nothing else; the call
    /// still has to survive tool resolution, the shell classifier, and the
    /// autonomy tier. Returning straight after the park would have let a
    /// channel that merely asks for confirmation authorize a `shell` command the
    /// tier blocks, or an acting tool under `readonly` — turning the channel's
    /// second-strictest setting into an override of the user's own autonomy
    /// choice. A refusal at any stage is still terminal, and `channel_approved`
    /// is carried forward so a later prompting stage does not ask twice.
    async fn authorize_tool(&self, call: &ToolCallRequest) -> TaResult<GateDecision> {
        self.authorize_tool_with_facts(call, None).await
    }

    async fn authorize_tool_with_context(
        &self,
        call: &ToolCallRequest,
        context: Option<&serde_json::Value>,
    ) -> TaResult<GateDecision> {
        self.authorize_tool_with_facts(call, context).await
    }

    /// Runs OpenHuman's prompt-injection detector over `text`.
    ///
    /// Every origin is screened, `User` included — the detector's primary
    /// production caller is the user-prompt path, and provenance is not trust.
    /// Both non-allow verdicts map to [`ScreenOutcome::Block`]: OpenHuman's
    /// `Review` verdict is already spelled `ReviewBlocked` on the enforcement
    /// side, so admitting it here would be this adapter inventing a permission
    /// the host does not grant.
    ///
    /// The reason string carries the verdict and the rule codes only — never
    /// the screened text, which is the thing suspected of being hostile.
    ///
    /// TODO(phase4): no [`ScreenOutcome::Redacted`] is ever returned. Producing
    /// one needs a free-text redactor; `tinymemory_api::safety::scan` detects spans but
    /// returns no rewritten string, and `approval::redact::scrub_paths` is
    /// private and shaped for JSON argument maps. Either lift a public
    /// `redact_text(&str) -> String` out of `approval::redact` or add one to
    /// `tinymemory_api::safety`, then map "PII found, injection clean" to `Redacted`.
    async fn screen_input(&self, text: &str, origin: ContentOrigin) -> TaResult<ScreenOutcome> {
        if !self.screens_input {
            return Ok(ScreenOutcome::Pass);
        }
        let source = match origin {
            ContentOrigin::User => "agent.user",
            ContentOrigin::Tool => "agent.tool_output",
            ContentOrigin::Web => "agent.web",
            ContentOrigin::Channel => "agent.channel",
            ContentOrigin::Agent => "agent.subagent",
            ContentOrigin::Stored => "agent.stored",
        };
        let decision = enforce_prompt_input(
            text,
            PromptEnforcementContext {
                source,
                request_id: None,
                user_id: None,
                session_id: None,
            },
        );

        match decision.action {
            PromptEnforcementAction::Allow => Ok(ScreenOutcome::Pass),
            PromptEnforcementAction::Blocked | PromptEnforcementAction::ReviewBlocked => {
                let codes: Vec<&str> = decision.reasons.iter().map(|r| r.code.as_str()).collect();
                tracing::warn!(
                    target: "tinyagents",
                    %source,
                    score = decision.score,
                    codes = %codes.join(","),
                    chars = decision.prompt_chars,
                    "[tinyagents::host::security] blocked untrusted input"
                );
                Ok(ScreenOutcome::block(format!(
                    "Content from '{source}' was rejected by the prompt-injection screen \
                     ({}). It has not been added to the conversation.",
                    if codes.is_empty() {
                        "no rule detail".to_string()
                    } else {
                        codes.join(", ")
                    }
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "security_gate_tests.rs"]
mod tests;
