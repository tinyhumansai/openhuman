use super::*;

const NATIVE_SECRET_GRANT_TOOL: &str = "native_secret_grant";

struct SecretPromptGuard(String);

impl Drop for SecretPromptGuard {
    fn drop(&mut self) {
        host().end_secret_prompt(&self.0);
    }
}

impl NativeEngine {
    async fn check_secret_master(&self, input: CheckInput) -> Result<Checked, EngineError> {
        use tinysecurity_bus::{EvaluateSecretRequest2, SecretOperation2, SecretRequest2};

        let registration_id = input
            .args
            .get("registration_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or_else(|| unavailable("native secret registration is unavailable"))?
            .to_owned();
        let proxy = self.proxy().await?;
        let client = crate::modules::security::Client::connect(&self.config)
            .await
            .map_err(unavailable)?;
        let generation = Self::generation(&proxy).await?;
        let scope = crate::modules::ops::security_secret_scope()
            .await
            .ok_or_else(|| unavailable("native secret scope unavailable"))?;
        let request = SecretRequest2 {
            registration_id: registration_id.clone(),
            call_id: input.call_id,
            ttl_ms: 30_000,
            operation: SecretOperation2::ResolveConfigMaster,
        };
        let _ordered = scope.lock().await;
        let audit = scope
            .correlation(generation)
            .await
            .map_err(EngineError::Unavailable)?;
        if !scope.begin_prompt(input.prompt) {
            return Err(unavailable(
                "another secret master request is already pending",
            ));
        }
        let _prompt = SecretPromptGuard(scope.scope_id().to_owned());
        let mut evaluate = EvaluateSecretRequest2 {
            request: request.clone(),
            audit,
        };
        let row = client
            .evaluate_secret2(&mut evaluate)
            .await
            .map_err(|error| unavailable(error.to_string()))?;
        if row.state != ApprovalLifecycle2::Pending
            || row.tool_name != NATIVE_SECRET_GRANT_TOOL
            || !scope.matches_handle(&row.binding.scope)
        {
            return Err(unavailable(
                "TinySecurity refused the native secret approval",
            ));
        }
        if !host().remember_secret_grant(request, row.request_id.clone(), row.expires_at_ms) {
            return Err(unavailable("native secret approval is already pending"));
        }
        Ok(Checked::Pending {
            request_id: row.request_id,
            expires_at_ms: row.expires_at_ms,
        })
    }
}

#[async_trait]
impl ApprovalEngine for NativeEngine {
    async fn check(&self, input: CheckInput) -> Result<Checked, EngineError> {
        if input.tool == NATIVE_SECRET_GRANT_TOOL {
            return self.check_secret_master(input).await;
        }
        if !is_host_tool_name2(&input.tool) {
            tracing::warn!(tool = %input.tool, "{LOG} the module cannot gate this tool name");
            return Err(unavailable(format!(
                "'{}' cannot be gated by the approval engine",
                input.tool
            )));
        }
        let proxy = self.proxy().await?;
        let subject = self.subject(&input.tenant, input.variant)?;
        let generation = Self::generation(&proxy).await?;
        let scope = scopes::ensure(
            &self.config,
            &proxy,
            subject.clone(),
            &input.posture,
            generation,
        )
        .await
        .map_err(EngineError::Unavailable)?;
        #[cfg(feature = "legacy-import")]
        if !crate::core::runtime::is_saas()
            && input.variant == Variant::Standard
            && !input.legacy_tool_grants.is_empty()
        {
            super::super::history_import::import_tool_grants(
                &proxy,
                &scope,
                &input.legacy_tool_grants,
            )
            .await;
        }
        self.sweep(&proxy, &scope, generation).await;

        let arguments_json =
            serde_json::to_string(&input.args).map_err(|e| unavailable(e.to_string()))?;
        let request = EvaluateRequest {
            caller: CallerContext {
                agent: subject.agent,
                thread: input.thread,
                flow: input.flow,
                call_id: input.call_id.clone(),
                origin: module_origin(input.origin),
                tier: AccessTier::Full,
            },
            call: ToolCall {
                tool: input.tool,
                arguments_json,
                checks: vec![Check::HostTool {}],
            },
        };
        if !host().begin_call(
            &input.call_id,
            CallEntry {
                prompt: input.prompt,
                scope: std::sync::Arc::clone(&scope),
                request: request.clone(),
            },
        ) {
            return Err(unavailable("the approval correlation is already in use"));
        }
        let _call = CallGuard(input.call_id.clone());
        let response = {
            let _guard = scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&scope, generation).await?;
            let ttl_ms = input
                .ttl
                .map(|ttl| u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX).max(1));
            let evaluated: Result<EvaluateApprovalResponse2, _> = proxy
                .call_confidential(
                    names::methods::EVALUATE_APPROVAL2,
                    (EvaluateApprovalRequest2 {
                        scope_id: scope.scope_id.clone(),
                        request: request.clone(),
                        ttl_ms,
                        summary: input.summary,
                        audit,
                    },),
                )
                .await;
            evaluated.map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused to evaluate the call");
                unavailable(format!("TinySecurity EvaluateApproval2 failed: {error}"))
            })?
        };
        let record = response.record;
        match response.decision.verdict {
            Verdict::Allow if record.state == ApprovalLifecycle2::Approved => {
                host().insert_request(&record.request_id, RequestEntry::new(scope, request, None));
                tracing::debug!(
                    request_id = %record.request_id,
                    attribution = record.attribution.as_deref().unwrap_or(""),
                    "{LOG} approved without a human"
                );
                Ok(Checked::Approved {
                    request_id: record.request_id,
                })
            }
            Verdict::RequireApproval { request_id }
                if request_id == record.request_id
                    && record.state == ApprovalLifecycle2::Pending =>
            {
                if host().request(&request_id).is_none() {
                    tracing::warn!(request_id = %request_id, "{LOG} pending without a delivered prompt");
                    return Err(unavailable("the approval prompt was not delivered"));
                }
                Ok(Checked::Pending {
                    request_id,
                    expires_at_ms: record.expires_at_ms,
                })
            }
            other => {
                tracing::debug!(verdict = ?other, "{LOG} the module did not approve the call");
                Err(unavailable("the approval engine refused the call"))
            }
        }
    }

    async fn decide(
        &self,
        request_id: &str,
        choice: Choice,
        label: &str,
    ) -> Result<Decided, EngineError> {
        let entry = host().request(request_id).ok_or(EngineError::NotPending)?;
        if entry.delivered.is_none() {
            // Approved without a human: there is nothing to decide.
            return Err(EngineError::NotPending);
        }
        let proxy = self.proxy().await?;
        let ticket = opaque();
        host().issue(
            ticket.clone(),
            Ticket::new(
                request_id.to_owned(),
                match choice {
                    Choice::Once => ApprovalChoice2::Once,
                    Choice::Deny => ApprovalChoice2::Deny,
                },
                ACTOR.to_owned(),
            ),
        );
        let decided = async {
            let _guard = entry.scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&entry.scope, generation).await?;
            proxy
                .call_confidential::<ApprovalRow2>(
                    names::methods::DECIDE2,
                    (DecideRequest2 {
                        scope_id: entry.scope.scope_id.clone(),
                        request_id: request_id.to_owned(),
                        decision_ticket: ticket.clone(),
                        audit,
                    },),
                )
                .await
                .map_err(|error| {
                    tracing::debug!(error = %error, "{LOG} the module refused the decision");
                    unavailable(format!("TinySecurity Decide2 failed: {error}"))
                })
        }
        .await;
        host().revoke(&ticket);
        match decided {
            Ok(row) => {
                if let Err(error) = entry.scope.rows.put_label(request_id, label).await {
                    tracing::debug!(error = %error, "{LOG} the decision label was not kept");
                }
                match row.state {
                    ApprovalLifecycle2::Approved => Ok(Decided::Approved),
                    ApprovalLifecycle2::Denied => Ok(Decided::Denied),
                    _ => Err(EngineError::NotPending),
                }
            }
            Err(error) => {
                if Self::settled(&entry, request_id).await {
                    Err(EngineError::NotPending)
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn reserve(&self, request_id: &str) -> Result<(), EngineError> {
        let entry = host().request(request_id).ok_or(EngineError::NotPending)?;
        let proxy = self.proxy().await?;
        let _guard = entry.scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&entry.scope, generation).await?;
        let action = Self::action(&entry, request_id, audit);
        let row: ApprovalRow2 = proxy
            .call_confidential(names::methods::CONSUME_APPROVAL2, (action,))
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused to reserve the call");
                unavailable(format!("TinySecurity ConsumeApproval2 failed: {error}"))
            })?;
        if row.state == ApprovalLifecycle2::Reserved {
            Ok(())
        } else {
            Err(unavailable("the approval was not reserved"))
        }
    }

    async fn record_execution(
        &self,
        request_id: &str,
        outcome: ExecutionOutcome,
    ) -> Result<(), EngineError> {
        let entry = host().request(request_id).ok_or(EngineError::NotPending)?;
        let proxy = self.proxy().await?;
        {
            let _guard = entry.scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&entry.scope, generation).await?;
            let _: ApprovalRow2 = proxy
                .call_confidential(
                    names::methods::RECORD_EXECUTION2,
                    (RecordExecutionRequest2 {
                        action: Self::action(&entry, request_id, audit),
                        outcome: module_outcome(outcome),
                    },),
                )
                .await
                .map_err(|error| {
                    tracing::warn!(error = %error, "{LOG} the module refused the execution record");
                    unavailable(format!("TinySecurity RecordExecution2 failed: {error}"))
                })?;
        }
        host().forget_request(request_id);
        Ok(())
    }

    async fn expire(&self, request_id: &str) -> Result<bool, EngineError> {
        let entry = host().request(request_id).ok_or(EngineError::NotPending)?;
        let proxy = self.proxy().await?;
        let expired = {
            let _guard = entry.scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&entry.scope, generation).await?;
            proxy
                .call_confidential::<ApprovalRow2>(
                    names::methods::EXPIRE_APPROVAL2,
                    (ApprovalRecordRequest2 {
                        scope_id: entry.scope.scope_id.clone(),
                        request_id: request_id.to_owned(),
                        audit,
                    },),
                )
                .await
        };
        match expired {
            Ok(row) => {
                host().forget_request(request_id);
                Ok(row.state == ApprovalLifecycle2::Expired)
            }
            Err(error) => {
                if Self::settled(&entry, request_id).await {
                    // Decided in the same instant: the caller honors the decision.
                    Ok(false)
                } else {
                    tracing::warn!(error = %error, "{LOG} the module refused to expire the request");
                    Err(unavailable(format!(
                        "TinySecurity ExpireApproval2 failed: {error}"
                    )))
                }
            }
        }
    }

    fn release(&self, request_id: &str) {
        host().forget_request(request_id);
    }

    async fn cancel(&self, request_id: &str) -> Result<(), EngineError> {
        let entry = host().request(request_id).ok_or(EngineError::NotPending)?;
        let proxy = self.proxy().await?;
        let cancelled = {
            let _guard = entry.scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&entry.scope, generation).await?;
            proxy
                .call_confidential::<ApprovalRow2>(
                    names::methods::CANCEL_APPROVAL2,
                    (ApprovalRecordRequest2 {
                        scope_id: entry.scope.scope_id.clone(),
                        request_id: request_id.to_owned(),
                        audit,
                    },),
                )
                .await
        };
        host().forget_request(request_id);
        cancelled.map(|_| ()).map_err(|error| {
            tracing::debug!(error = %error, "{LOG} the module did not cancel the request");
            unavailable(format!("TinySecurity CancelApproval2 failed: {error}"))
        })
    }

    async fn is_decided(&self, request_id: &str) -> Result<Option<bool>, EngineError> {
        super::super::view::row_decided(&self.config, request_id)
            .await
            .map_err(|error| unavailable(error.to_string()))
    }

    async fn history(&self, limit: usize) -> Result<Vec<ApprovalAuditEntry>, EngineError> {
        super::super::view::decided_history(&self.config, limit)
            .await
            .map_err(|error| unavailable(error.to_string()))
    }

    async fn grant_flow_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        flow_id: &str,
        tool: &str,
    ) -> Result<(), EngineError> {
        let proxy = self.proxy().await?;
        let scope = self.flow_scope(&proxy, tenant).await?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: FlowTrustReceipt2 = proxy
            .call_confidential(
                names::methods::GRANT_FLOW_TOOL2,
                (GrantFlowToolRequest2 {
                    scope_id: scope.scope_id.clone(),
                    flow_id: flow_id.to_owned(),
                    tool: tool.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused the flow trust");
                unavailable(format!("TinySecurity GrantFlowTool2 failed: {error}"))
            })?;
        tracing::debug!(outcome = ?receipt.outcome, "{LOG} flow trust granted");
        Ok(())
    }

    async fn grant_tool_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        tool: &str,
    ) -> Result<(), EngineError> {
        if !is_host_tool_name2(tool) {
            return Err(unavailable("TinySecurity refuses this tool name"));
        }
        let proxy = self.proxy().await?;
        let subject = self.subject(tenant, variant)?;
        let generation = Self::generation(&proxy).await?;
        let scope = scopes::ensure(
            &self.config,
            &proxy,
            subject,
            &Posture {
                require_approval: true,
                ..Default::default()
            },
            generation,
        )
        .await
        .map_err(EngineError::Unavailable)?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: ToolTrustReceipt2 = proxy
            .call_confidential(
                names::methods::GRANT_TOOL2,
                (GrantToolRequest2 {
                    scope_id: scope.scope_id.clone(),
                    tool: tool.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| unavailable(format!("TinySecurity GrantTool2 failed: {error}")))?;
        if receipt.scope_id != scope.scope_id {
            return Err(unavailable(
                "TinySecurity returned a receipt for another scope",
            ));
        }
        Ok(())
    }

    async fn tool_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        posture: Posture,
    ) -> Result<Vec<crate::security::approval::types::ToolTrustEntry>, EngineError> {
        let proxy = self.proxy().await?;
        let subject = self.subject(tenant, variant)?;
        let generation = Self::generation(&proxy).await?;
        let scope = scopes::ensure(&self.config, &proxy, subject, &posture, generation)
            .await
            .map_err(EngineError::Unavailable)?;
        let listed: ToolTrustList2 = proxy
            .call_confidential(
                names::methods::LIST_TOOL_TRUST2,
                (ListToolTrustRequest2 {
                    scope_id: scope.scope_id.clone(),
                },),
            )
            .await
            .map_err(|error| unavailable(format!("TinySecurity ListToolTrust2 failed: {error}")))?;
        if listed.entries.len() > 4096 {
            return Err(unavailable("TinySecurity returned too many tool grants"));
        }
        Ok(listed
            .entries
            .into_iter()
            .map(|entry| crate::security::approval::types::ToolTrustEntry {
                tool: entry.tool,
                request_id: entry.request_id,
                granted_at_ms: entry.granted_at_ms,
            })
            .collect())
    }

    async fn revoke_tool_trust_by_id(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        posture: Posture,
        request_id: &str,
    ) -> Result<bool, EngineError> {
        if request_id.is_empty() || request_id.len() > 128 {
            return Err(unavailable("invalid TinySecurity tool grant id"));
        }
        let proxy = self.proxy().await?;
        let subject = self.subject(tenant, variant)?;
        let generation = Self::generation(&proxy).await?;
        let scope = scopes::ensure(&self.config, &proxy, subject, &posture, generation)
            .await
            .map_err(EngineError::Unavailable)?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: ToolTrustReceipt2 = proxy
            .call_confidential(
                names::methods::REVOKE_TOOL_BY_ID2,
                (RevokeToolByIdRequest2 {
                    scope_id: scope.scope_id.clone(),
                    request_id: request_id.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| {
                unavailable(format!("TinySecurity RevokeToolById2 failed: {error}"))
            })?;
        if receipt.scope_id != scope.scope_id {
            return Err(unavailable(
                "TinySecurity returned a receipt for another scope",
            ));
        }
        Ok(receipt.outcome == tinysecurity_bus::ToolTrustOutcome2::Revoked)
    }

    async fn revoke_flow_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        flow_id: &str,
        tool: &str,
    ) -> Result<bool, EngineError> {
        let proxy = self.proxy().await?;
        let scope = self.flow_scope(&proxy, tenant).await?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: FlowTrustReceipt2 = proxy
            .call_confidential(
                names::methods::REVOKE_FLOW_TOOL2,
                (RevokeFlowToolRequest2 {
                    scope_id: scope.scope_id.clone(),
                    flow_id: flow_id.to_owned(),
                    tool: tool.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused to revoke flow trust");
                unavailable(format!("TinySecurity RevokeFlowTool2 failed: {error}"))
            })?;
        Ok(receipt.outcome == FlowTrustOutcome2::Revoked)
    }

    async fn flow_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        flow_id: &str,
    ) -> Result<Vec<String>, EngineError> {
        let proxy = self.proxy().await?;
        let scope = self.flow_scope(&proxy, tenant).await?;
        let listed: FlowTrustList2 = proxy
            .call_confidential(
                names::methods::LIST_FLOW_TRUST2,
                (ListFlowTrustRequest2 {
                    scope_id: scope.scope_id.clone(),
                    flow_id: Some(flow_id.to_owned()),
                },),
            )
            .await
            .map_err(|error| unavailable(format!("TinySecurity ListFlowTrust2 failed: {error}")))?;
        let mut tools: Vec<String> = listed.entries.into_iter().map(|e| e.tool).collect();
        tools.sort();
        tools.dedup();
        Ok(tools)
    }
}
