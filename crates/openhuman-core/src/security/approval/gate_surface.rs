impl Parked {
    fn take_waiter(&self, request_id: &str) -> Option<oneshot::Sender<ApprovalDecision>> {
        self.waiters.lock().remove(request_id)
    }

    fn evict_waiter(&self, request_id: &str) {
        self.waiters.lock().remove(request_id);
    }

    /// Record the routing correlation for a newly-parked request.
    fn insert_request_route(&self, request_id: &str, route: RequestRoute) {
        self.request_routes
            .lock()
            .insert(request_id.to_string(), route);
    }

    /// Remove and return the routing correlation for `request_id`, if any.
    /// Consumed exactly once per request — by whichever path resolves the
    /// decision first (`decide`, a TTL timeout, or a dropped channel).
    fn take_request_route(&self, request_id: &str) -> Option<RequestRoute> {
        self.request_routes.lock().remove(request_id)
    }

    /// Drop the thread → request mapping when it still belongs to this request.
    fn clear_thread(&self, thread_key: &Option<String>, request_id: &str) {
        if let Some(key) = thread_key {
            self.clear_thread_route_if_owned(key, request_id);
        }
    }

    /// Drop the thread → request mapping **only if** it still points at
    /// `request_id`. Used by [`CancelGuard::drop`] on external teardown, where a
    /// replacement turn may have already parked a new approval on the same
    /// thread and overwritten the entry; clearing unconditionally would delete
    /// the *new* request's routing (#4774).
    fn clear_thread_route_if_owned(&self, thread_key: &str, request_id: &str) {
        let mut map = self.thread_to_request.lock();
        if map.get(thread_key).is_some_and(|rid| rid == request_id) {
            map.remove(thread_key);
        }
    }
}

impl Surface {
    /// The request the engine delivered for this call, once it did.
    fn delivered_request(&self) -> Option<String> {
        self.delivered.lock().clone()
    }
}

fn instant(ms: u64) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_millis_opt(i64::try_from(ms).unwrap_or(i64::MAX))
        .single()
        .unwrap_or_default()
}

/// How a gated call is asked, beyond what its turn says.
#[derive(Clone, Copy, Debug)]
struct Mode {
    /// An explicit one-time decision for this exact action; no shortcut applies.
    forced: bool,
    /// The caller learns the request id and reports how the call ended, so the
    /// approval is reserved for execution and its outcome recorded.
    track: bool,
}

impl Mode {
    fn forced(forced: bool) -> Self {
        Self {
            forced,
            track: true,
        }
    }

    fn untracked(mut self) -> Self {
        self.track = false;
        self
    }
}

/// An eight-hex-character correlation for one gated call. The engine hashes
/// identifiers with a function that leaves a value of exactly this shape
/// unchanged, so the prompt callback finds the call by what the row carries.
fn fresh_call_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
}

#[async_trait::async_trait]
impl PromptSurface for Surface {
    /// The engine made a request durable and asks for it to be shown: register
    /// the waiter and the routing, then publish the events the surfaces read.
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        let ctx = &self.context;
        if self.delivered.lock().as_deref() == Some(delivery.request_id.as_str()) {
            tracing::debug!(
                request_id = %delivery.request_id,
                "[approval::gate] prompt already shown for this request"
            );
            return Ok(());
        }
        let Some(waiter) = self.waiter.lock().take() else {
            return Err("this call already has a prompt".into());
        };
        let request_id = delivery.request_id.clone();
        let expires_at = Some(instant(delivery.expires_at_ms));
        let pending = PendingApproval {
            request_id: request_id.clone(),
            tool_name: delivery.tool_name.clone(),
            action_summary: delivery.summary.clone(),
            args_redacted: delivery.args_redacted.clone(),
            created_at: instant(delivery.created_at_ms),
            expires_at,
            source_context: ctx.source_context.clone(),
            tool_call_id: ctx.tool_call_id.clone(),
            agent_id: ctx.agent_id.clone(),
        };
        let parked = &self.parked;
        // Register the waiter BEFORE the request can be decided so a fast
        // `approval_decide` cannot mark it approved while no waiter exists —
        // would otherwise leave the parked call to time out and return `Deny`
        // incorrectly. (CodeRabbit review on PR #2149.) The whole registration
        // and its publication run inside the instance barrier.
        let register = |workspace| {
            parked.waiters.lock().insert(request_id.clone(), waiter);
            // Record the thread → request mapping so an inbound chat reply on this
            // thread can be routed to `approval_decide` (see web channel ingress).
            if let Some(thread_key) = ctx.thread_key.as_ref() {
                parked
                    .thread_to_request
                    .lock()
                    .insert(thread_key.clone(), request_id.clone());
            }
            // Record the full routing correlation (thread/client/tool_call_id) so
            // whichever path resolves this request's decision — `decide()`, the
            // TTL timeout, or a dropped decision channel — can mirror
            // it onto `ApprovalDecided` without re-deriving it from ambient
            // task-locals that may no longer be in scope by then.
            parked.insert_request_route(
                &request_id,
                RequestRoute {
                    thread_id: ctx.chat_thread_id.clone(),
                    client_id: ctx.chat_client_id.clone(),
                    tool_call_id: ctx.tool_call_id.clone(),
                    forced: ctx.forced,
                    agent_id: ctx.agent_id.clone(),
                    variant: ctx.variant,
                    approval_scope: ctx.approval_scope.clone(),
                    thread_key: ctx.thread_key.clone(),
                    detached: ctx.detached,
                    tenant: ctx.tenant.clone(),
                    pending: pending.clone(),
                },
            );
            tracing::info!(
                request_id = %request_id,
                tool = %delivery.tool_name,
                thread_id = ctx.chat_thread_id.as_deref().unwrap_or("<none>"),
                client_id = ctx.chat_client_id.as_deref().unwrap_or("<none>"),
                agent_id = ctx.agent_id.as_deref().unwrap_or("<none>"),
                "[approval::gate] publishing ApprovalRequested (surface fires only if thread_id+client_id are both set)"
            );
            BUS.publish(DomainEvent::ApprovalRequested {
                request_id: request_id.clone(),
                tool_name: delivery.tool_name.clone(),
                action_summary: delivery.summary.clone(),
                args_redacted: delivery.args_redacted.clone(),
                thread_id: ctx.chat_thread_id.clone(),
                client_id: ctx.chat_client_id.clone(),
                tool_call_id: ctx.tool_call_id.clone(),
                expires_at: expires_at.map(|t| t.to_rfc3339()),
                agent_id: ctx.agent_id.clone(),
            });
            // Publish every surface inside the same registration barrier.
            // Workspace lookup cannot delay a flow surface after the generic
            // request; scoped removal and decisions wait for every surface.
            if let Some(ApprovalSourceContext::Flow {
                flow_id, run_id, ..
            }) = &ctx.source_context
            {
                tracing::info!(request_id = %request_id, flow_id = %flow_id,
                    run_id = %run_id, tool = %delivery.tool_name,
                    "[approval::gate] flow-origin park — surfacing flow_approval_request + notification");
                BUS.publish(DomainEvent::FlowApprovalRequested {
                    request_id: request_id.clone(),
                    flow_id: flow_id.clone(),
                    run_id: run_id.clone(),
                    tool_name: delivery.tool_name.clone(),
                    summary: delivery.summary.clone(),
                    agent_id: ctx.agent_id.clone(),
                });
                publish_flow_gate_notification(
                    &request_id,
                    flow_id,
                    run_id,
                    &delivery.tool_name,
                    &delivery.summary,
                    workspace,
                );
            }
        };
        let registered = super::flow_surface::register_after_workspace(
            ctx.approval_scope.as_deref(),
            async {
                if !matches!(&ctx.source_context, Some(ApprovalSourceContext::Flow { .. })) {
                    return None;
                }
                // Resolve before registration; never hold the barrier across
                // asynchronous workspace I/O. An unbound live notification
                // still surfaces the request when workspace lookup fails.
                match crate::config::active_workspace_snapshot().await {
                    Ok((dir, revision)) => Some((crate::config::workspace_handle(&dir), revision)),
                    Err(error) => {
                        tracing::warn!(request_id = %delivery.request_id,
                            "[approval::gate] could not resolve the active workspace for the flow approval notification ({error}); publishing it unbound");
                        None
                    }
                }
            },
            register,
        )
        .await;
        match registered {
            Err(resolution) => Err(format!("Agent approval registration closed: {resolution}")),
            Ok(()) => {
                *self.delivered.lock() = Some(delivery.request_id.clone());
                Ok(())
            }
        }
    }
}


/// The prompt surface of a tenant that has none: any request that reaches it is
/// refused, so the engine can never park a call nobody can answer.
struct NoApprovalSurface;

#[async_trait::async_trait]
impl PromptSurface for NoApprovalSurface {
    async fn deliver(&self, _delivery: Delivery) -> Result<(), String> {
        Err("a SaaS tenant has no approval surface".into())
    }
}
