use super::*;

pub(super) struct NativeSubagentSecurityMiddleware {
    gate: Arc<OpenHumanSecurityGate>,
    agent_id: String,
}

impl NativeSubagentSecurityMiddleware {
    pub(super) fn new(gate: Arc<OpenHumanSecurityGate>, agent_id: String) -> Self {
        Self { gate, agent_id }
    }
}

#[async_trait]
impl ToolMiddleware<(), OpenHumanRunContext> for NativeSubagentSecurityMiddleware {
    fn name(&self) -> &str {
        "openhuman_security_gate"
    }

    async fn wrap_tool(
        &self,
        ctx: &tinyagents_harness::context::RunContext<OpenHumanRunContext>,
        state: &(),
        call: tinyinference_llm::tool::ToolCall,
        next: ToolHandler<'_, (), OpenHumanRunContext>,
    ) -> tinyagents_harness::error::Result<MiddlewareToolOutcome> {
        let call_id = call.id.to_string();
        let request = ToolCallRequest::new(
            call.name.clone(),
            call.arguments.clone(),
            self.agent_id.clone(),
        )
        .with_call_id(call_id.clone());
        let decision = match self.gate.authorize_tool(&request).await {
            Ok(decision) => decision,
            Err(error) => {
                tracing::error!(tool = %call.name, %error, "native child authorization failed closed");
                return Ok(tinytools::ToolResult::error(
                    "Tool authorization failed because the security check could not complete.",
                )
                .into());
            }
        };
        match decision {
            GateDecision::Allow | GateDecision::Prompted { approved: true } => {}
            GateDecision::Deny { reason } => {
                return Ok(tinytools::ToolResult::error(reason).into());
            }
            GateDecision::Prompted { approved: false } => {
                return Ok(tinytools::ToolResult::error("Approval was not granted.").into());
            }
        }

        let mut audit = self
            .gate
            .take_audit_request_id(&call_id)
            .map(NativeSecurityAudit::new);
        let result = next.run(ctx, state, call).await;
        if let Some(audit) = &mut audit {
            match &result {
                Ok(MiddlewareToolOutcome::Result(result)) if result.is_error => {
                    audit.record(
                        crate::security::approval::ExecutionOutcome::Failure,
                        Some(result.output()),
                    );
                }
                Ok(_) => audit.record(crate::security::approval::ExecutionOutcome::Success, None),
                Err(error) => audit.record(
                    crate::security::approval::ExecutionOutcome::Failure,
                    Some(error.to_string()),
                ),
            }
        }
        result
    }
}

struct NativeSecurityAudit {
    request_id: String,
    recorded: bool,
}

impl NativeSecurityAudit {
    fn new(request_id: String) -> Self {
        Self {
            request_id,
            recorded: false,
        }
    }

    fn record(
        &mut self,
        execution: crate::security::approval::ExecutionOutcome,
        error: Option<String>,
    ) {
        if let Some(gate) = crate::security::approval::ApprovalGate::try_global() {
            gate.record_execution(&self.request_id, execution, error.as_deref());
        }
        self.recorded = true;
    }
}

impl Drop for NativeSecurityAudit {
    fn drop(&mut self) {
        if !self.recorded {
            self.record(
                crate::security::approval::ExecutionOutcome::Failure,
                Some("tool execution ended before reporting a result".to_string()),
            );
        }
    }
}
