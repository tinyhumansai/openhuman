use super::*;

impl OpenHumanSecurityGate {
    pub(super) async fn authorize_tool_with_facts(
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
        // actions. Keep Cron turns read-only, except that a headless root which
        // owns descendant settlement may use the canonical spawn tool as
        // internal orchestration control. This does not permit effectful spawn
        // implementations, shell, or any other acting tool. The check precedes
        // shell and approval shortcuts, including when approvals are disabled.
        let is_cron_turn = matches!(
            self.turn_origin(),
            Some(
                crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
                    source: crate::core::runtime::turn_origin::TrustedAutomationSource::Cron,
                    ..
                }
            )
        );
        let is_headless_spawn_control =
            self.headless_descendant_settlement && call.tool_name == "spawn_subagent";
        let is_cron_denial = is_cron_turn
            && (call.tool_name == SHELL_TOOL
                || tool.external_effect_with_args(&call.arguments)
                || (tool.permission_level_with_args(&call.arguments) > PermissionLevel::ReadOnly
                    && !is_headless_spawn_control));
        if is_cron_denial {
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
            let _ = authorization_context;
            let Some(command) = call
                .arguments
                .get("command")
                .and_then(serde_json::Value::as_str)
            else {
                return Ok(GateDecision::deny(format!(
                    "{} Shell command is missing or invalid.",
                    tinytools::status::POLICY_BLOCKED_MARKER
                )));
            };
            let verdict = match policy
                .check_command(crate::security::policy::CommandQuery {
                    command,
                    declared: None,
                    flow: crate::security::policy::CommandFlow::Harness,
                    stage: crate::security::policy::CommandStage::Authorize,
                })
                .await
            {
                Ok(verdict) => verdict,
                Err(reason) => return Ok(GateDecision::deny(reason.to_string())),
            };
            return Ok(match verdict.gate {
                PolicyGateDecision::Block => GateDecision::deny(
                    "[policy-blocked] Command category is not permitted by TinySecurity.",
                ),
                PolicyGateDecision::Prompt => self.park_once(call, channel_approved).await,
                PolicyGateDecision::Allow => self.settled(channel_approved),
            });
        }

        let required = tool.permission_level_with_args(&call.arguments);
        let external_effect = tool.external_effect_with_args(&call.arguments);
        let class = match crate::security::policy::host_action_class(required, external_effect)
            .await
        {
            Ok(class) => match class {
                tinysecurity_bus::CommandClass::Read => crate::security::policy::CommandClass::Read,
                tinysecurity_bus::CommandClass::Write => {
                    crate::security::policy::CommandClass::Write
                }
                tinysecurity_bus::CommandClass::Network => {
                    crate::security::policy::CommandClass::Network
                }
                tinysecurity_bus::CommandClass::Install => {
                    crate::security::policy::CommandClass::Install
                }
                tinysecurity_bus::CommandClass::Destructive => {
                    crate::security::policy::CommandClass::Destructive
                }
            },
            Err(error) => {
                tracing::warn!(target: "tinyagents", tool = %call.tool_name, "TinySecurity action classification unavailable: {error}");
                return Ok(GateDecision::deny(
                    "[policy-blocked] TinySecurity action classification unavailable.",
                ));
            }
        };
        let tier_gate = match policy.command_class_gate(class).await {
            Ok(gate) => gate,
            Err(reason) => return Ok(GateDecision::deny(reason.to_string())),
        };
        if tier_gate == PolicyGateDecision::Block {
            return Ok(GateDecision::deny(format!(
                "{} Security policy blocks this action.",
                tinytools::status::POLICY_BLOCKED_MARKER
            )));
        }
        let mut approved = channel_approved;
        if tier_gate == PolicyGateDecision::Prompt && !desktop_approval_disabled {
            let decision = self.park_once(call, channel_approved).await;
            if !decision.is_allowed() {
                return Ok(decision);
            }
            approved = true;
        }

        // Sync-only TinyTools filesystem and network gates read this cached
        // module answer. Their `record_action` call does not keep a host ledger.
        let budget: Result<(), String> = if class == crate::security::policy::CommandClass::Read {
            match policy.peek_action_budget().await {
                Ok(tinysecurity_bus::BudgetDecision2::RateLimited { .. }) => {
                    Err("Rate limit exceeded: action budget exhausted".to_owned())
                }
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            policy.spend_action(&call.tool_name).await
        };
        if let Err(reason) = budget {
            return Ok(GateDecision::deny(reason));
        }
        channel_approved = approved;

        tracing::debug!(
            target: "tinyagents",
            tool = %call.tool_name,
            "[tinyagents::host::security] allowed"
        );
        Ok(self.settled(channel_approved))
    }
}
