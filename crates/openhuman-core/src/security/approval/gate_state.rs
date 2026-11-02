use crate::core::runtime::DispatchCtx;

impl ApprovalGate {
    /// Check the authenticated dispatch against the live parked request before
    /// allowing an approval decision. A process-scoped UI may project into one
    /// agent's request only by naming that exact agent; an agent-scoped
    /// dispatch cannot decide for another agent or the process.
    pub(crate) fn validate_decision_context(
        &self,
        request_id: &str,
        dispatch_tenant: &crate::core::runtime::Tenant,
        requested_agent: Option<&str>,
    ) -> anyhow::Result<(crate::core::runtime::Tenant, Variant)> {
        let route = self
            .parked
            .request_routes
            .lock()
            .get(request_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("approval request is not parked in this host session"))?;
        validate_decision_tenant(
            dispatch_tenant,
            requested_agent,
            &route.tenant,
            request_id,
        )?;
        Ok((route.tenant, route.variant))
    }

    /// Persist a whole-tool grant under the parked request's exact subject.
    /// Called only after `decide` has resolved the current request.
    pub(crate) fn grant_tool_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        tool: &str,
    ) -> anyhow::Result<()> {
        let tenant = tenant.clone();
        let tool = tool.to_owned();
        self.run_engine(|engine| async move {
            engine.grant_tool_trust(&tenant, variant, &tool).await
        })?
        .map_err(|error| anyhow::anyhow!("could not save the whole-tool grant: {error}"))
    }

    /// List whole-tool grants in the exact standard approval scope.
    pub(crate) fn list_tool_trust(
        &self,
        ctx: &DispatchCtx,
    ) -> anyhow::Result<Vec<super::types::ToolTrustEntry>> {
        let tenant = ctx
            .tenant()
            .map_err(|error| anyhow::anyhow!("tool grant listing has no tenant: {error}"))?;
        let posture = self.posture(ctx, Variant::Standard);
        self.run_engine(|engine| async move {
            engine.tool_trust(&tenant, Variant::Standard, posture).await
        })?
            .map_err(|error| anyhow::anyhow!("could not list whole-tool grants: {error}"))
    }

    /// Revoke one whole-tool grant by the opaque ID returned from
    /// [`Self::list_tool_trust`]. The module checks that the ID belongs to this
    /// tenant's registered scope.
    pub(crate) fn revoke_tool_trust_by_id(
        &self,
        ctx: &DispatchCtx,
        request_id: &str,
    ) -> anyhow::Result<bool> {
        let tenant = ctx
            .tenant()
            .map_err(|error| anyhow::anyhow!("tool grant revocation has no tenant: {error}"))?;
        let (posture, request_id) = (self.posture(ctx, Variant::Standard), request_id.to_owned());
        self.run_engine(|engine| async move {
            engine
                .revoke_tool_trust_by_id(&tenant, Variant::Standard, posture, &request_id)
                .await
        })?
        .map_err(|error| anyhow::anyhow!("could not revoke whole-tool grant: {error}"))
    }

    /// Run an engine call from synchronous code. The decision side of the gate is
    /// synchronous by contract (the embedding API, an agent's teardown, the
    /// harness's record hook), so the engine's async call runs on the shared
    /// blocking bridge instead of the caller's runtime, which cannot be blocked
    /// on a current-thread runtime.
    fn run_engine<T, F, Fut>(&self, op: F) -> anyhow::Result<T>
    where
        F: FnOnce(SharedEngine) -> Fut,
        Fut: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let future = op(Arc::clone(&self.engine));
        crate::storage::block_on_anyhow(async move { Ok(future.await) })
    }

    /// Write the *terminal* status of a tool call onto its approval
    /// audit row: how the call the gate reserved ended.
    ///
    /// Logs (but does not propagate) write errors to preserve the result of a
    /// tool that has already run. A failed write also latches the gate closed
    /// for later external admissions.
    ///
    /// `error` is the (already sanitized) failure text of the call. The
    /// engine records the outcome class only; the text is never stored, which
    /// keeps whatever it quotes out of the durable log.
    pub fn record_execution(
        &self,
        request_id: &str,
        outcome: ExecutionOutcome,
        error: Option<&str>,
    ) {
        let _ = self.record_execution_required(request_id, outcome, error);
    }

    /// Record a terminal result required by the security audit contract.
    /// Failure is returned to internal callers and permanently fails closed
    /// for later external-effect admissions in this process.
    pub(crate) fn record_execution_required(
        &self,
        request_id: &str,
        outcome: ExecutionOutcome,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = request_id.to_owned();
        let recorded = self
            .run_engine(|engine| async move { engine.record_execution(&id, outcome).await })
            .map_err(|bridge| anyhow::anyhow!("approval audit engine bridge failed: {bridge}"))
            .and_then(|result| result.map_err(anyhow::Error::from));
        match recorded {
            Ok(()) => {
                tracing::debug!(
                request_id = %request_id,
                outcome = outcome.as_str(),
                had_error = error.is_some(),
                "[approval::gate] recorded terminal execution"
                );
                Ok(())
            }
            Err(err) => {
                self.audit_failure.store(true, Ordering::SeqCst);
                tracing::error!(
                    request_id = %request_id,
                    outcome = outcome.as_str(),
                    error = %err,
                    "[approval::gate] required terminal audit write failed; later external admissions fail closed"
                );
                Err(err)
            }
        }
    }

    /// Apply a user decision. Returns the now-decided
    /// [`PendingApproval`] when the request was parked in this process.
    ///
    /// `Ok(None)` means there is nothing to decide: the request was already
    /// decided, expired or cancelled, or was never parked here (an earlier launch
    /// left it; the engine cannot resume another instance's request).
    pub fn decide(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> anyhow::Result<Option<PendingApproval>> {
        let Some(route) = self.parked.request_routes.lock().get(request_id).cloned() else {
            return Ok(None);
        };
        // A removed agent's requests are denied by its teardown; a decision
        // racing it must not approve one.
        if route
            .approval_scope
            .as_deref()
            .is_some_and(super::ApprovalScope::is_closed)
        {
            return Ok(None);
        }
        if !matches!(
            decision,
            ApprovalDecision::ApproveOnce | ApprovalDecision::Deny
        ) && route.forced
        {
            anyhow::bail!("this action requires a one-time approval or denial");
        }
        let choice = if decision.is_approve() {
            Choice::Once
        } else {
            Choice::Deny
        };
        let id = request_id.to_owned();
        let label = decision.as_str();
        match self.run_engine(|engine| async move { engine.decide(&id, choice, label).await })? {
            Ok(_) => {}
            Err(EngineError::NotPending) => return Ok(None),
            Err(EngineError::Refused) => {
                anyhow::bail!("the approval engine refused the decision")
            }
            Err(EngineError::Unavailable(message)) => {
                anyhow::bail!("the approval engine could not apply the decision: {message}")
            }
        }
        // `ApproveAlwaysForTool` persistence is handled by the `approval_decide`
        // RPC handler after this call has been released. The module records the
        // whole-tool grant under the parked request's exact subject.
        if let Some(tx) = self.parked.take_waiter(request_id) {
            let _ = tx.send(decision);
        }
        // Routing (thread/client/tool_call_id) was recorded at park time —
        // see `Surface::deliver` — so a decision made after the in-memory
        // waiter already resolved (e.g. via the TTL/channel-drop paths in
        // `gate_intercept_decision.rs`) still reports `None` here, which is
        // correct: this fn only fires for a live `decide()` call.
        let route = self.parked.take_request_route(request_id).unwrap_or(route);
        BUS.publish(DomainEvent::ApprovalDecided {
            request_id: request_id.to_string(),
            tool_name: route.pending.tool_name.clone(),
            decision: decision.as_str().to_string(),
            thread_id: route.thread_id.clone(),
            client_id: route.client_id.clone(),
            tool_call_id: route.tool_call_id.clone(),
            resolution: None,
            agent_id: route.agent_id.clone(),
        });
        Ok(Some(route.pending))
    }

    /// [`Self::decide`] on behalf of `agent`: refuses with
    /// [`ApprovalError::WrongAgent`] when the request was parked by any other
    /// agent (or by the process itself), so one agent cannot answer another's
    /// approval.
    pub fn decide_for_agent(
        &self,
        agent: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> anyhow::Result<Option<PendingApproval>> {
        let owner = self
            .parked
            .request_routes
            .lock()
            .get(request_id)
            .map(|route| route.agent_id.clone());
        if let Some(owner) = owner {
            if owner.as_deref() != Some(agent) {
                tracing::warn!(
                    request_id = %request_id,
                    agent_id = %agent,
                    owner = owner.as_deref().unwrap_or("<process>"),
                    "[approval::gate] decision refused: request belongs to another agent"
                );
                return Err(ApprovalError::WrongAgent {
                    request_id: request_id.to_string(),
                }
                .into());
            }
        }
        self.decide(request_id, decision)
    }

    /// Deny every undecided request `agent` parked, resolving its waiters.
    /// Returns how many requests were denied.
    pub fn deny_all_for_agent(
        &self,
        _ctx: &DispatchCtx,
        agent: &str,
        resolution: &str,
    ) -> anyhow::Result<usize> {
        let request_ids: Vec<String> = self
            .parked
            .request_routes
            .lock()
            .iter()
            .filter(|(_, route)| route.agent_id.as_deref() == Some(agent))
            .map(|(id, _)| id.clone())
            .collect();
        let mut denied = 0;
        for request_id in request_ids {
            let Some(route) = self.parked.take_request_route(&request_id) else {
                continue;
            };
            let id = request_id.clone();
            match self.run_engine(|engine| async move { engine.cancel(&id).await }) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::debug!(
                    request_id = %request_id,
                    error = %error,
                    "[approval::gate] the engine did not cancel a removed agent's request"
                ),
                Err(error) => tracing::debug!(
                    request_id = %request_id,
                    error = %error,
                    "[approval::gate] cancelling a removed agent's request could not run"
                ),
            }
            denied += 1;
            if let Some(tx) = self.parked.take_waiter(&request_id) {
                let _ = tx.send(ApprovalDecision::Deny);
            }
            if let Some(thread_key) = route.thread_key.as_deref() {
                self.parked
                    .clear_thread_route_if_owned(thread_key, &request_id);
            }
            BUS.publish(DomainEvent::ApprovalDecided {
                request_id: request_id.clone(),
                tool_name: route.pending.tool_name.clone(),
                decision: ApprovalDecision::Deny.as_str().to_string(),
                thread_id: route.thread_id.clone(),
                client_id: route.client_id.clone(),
                tool_call_id: route.tool_call_id.clone(),
                resolution: Some(resolution.to_string()),
                agent_id: Some(agent.to_string()),
            });
        }
        tracing::info!(
            agent_id = %agent,
            denied,
            resolution,
            "[approval::gate] denied every pending request of the agent"
        );
        Ok(denied)
    }

    /// Classify a [`Self::decide`] miss — i.e. when `decide` returned
    /// `Ok(None)` because the request is no longer pending here.
    ///
    /// - [`DecideMiss::AlreadyResolved`] — the request exists but was **already
    ///   decided, expired, cancelled, or closed for removal**.
    ///   This is the benign
    ///   double-tap / two-operator / expiry-while-live race the inline-approvals
    ///   design spec classifies as benign (TAURI-RUST-5EH).
    /// - [`DecideMiss::NeverRegistered`] — no request was ever stored for this
    ///   request_id. That is a genuine lost registration (a core restart dropped
    ///   the parked future before storing, or a stray id) and must stay a
    ///   Sentry signal.
    ///
    /// We disambiguate by asking the engine whether it stored the request and
    /// whether it left `Pending`. If the lookup itself errors we conservatively
    /// keep the event visible (`NeverRegistered`) rather than silently demoting.
    pub fn classify_decide_miss(&self, request_id: &str) -> DecideMiss {
        if self
            .request_scope(request_id)
            .is_some_and(|scope| scope.is_closed())
        {
            return DecideMiss::AlreadyResolved;
        }
        let id = request_id.to_owned();
        let known = self
            .run_engine(|engine| async move { engine.is_decided(&id).await })
            .map_err(|bridge| EngineError::Unavailable(bridge.to_string()))
            .and_then(|answer| answer);
        match known {
            Ok(Some(true)) => DecideMiss::AlreadyResolved,
            Ok(Some(false)) => DecideMiss::NeverRegistered,
            Ok(None) => DecideMiss::NeverRegistered,
            Err(err) => {
                tracing::warn!(
                    request_id = %request_id,
                    error = %err,
                    "[approval::gate] classify_decide_miss: lookup failed; treating as never-registered (keep visible)"
                );
                DecideMiss::NeverRegistered
            }
        }
    }

    /// The requests this tenant has parked and not yet seen decided, oldest
    /// first. A SaaS profile sees every agent's; anything else sees only its own
    /// agent's (the process, for the default orchestrator).
    pub fn list_pending(&self) -> anyhow::Result<Vec<PendingApproval>> {
        let tenant = super::gate::current_route_tenant();
        Ok(self.pending_where(|route| {
            route.tenant.profile == tenant.profile
                && (tenant.profile.is_some() || route.tenant.agent == tenant.agent)
        }))
    }

    /// [`Self::list_pending`] narrowed to `agent`'s requests; `None` lists the
    /// process's own.
    pub fn list_pending_for_agent(
        &self,
        ctx: &DispatchCtx,
        agent: Option<&str>,
    ) -> anyhow::Result<Vec<PendingApproval>> {
        let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
        Ok(self.pending_where(|route| {
            route.tenant.profile == profile && route.pending.belongs_to(agent)
        }))
    }

    fn pending_where(&self, keep: impl Fn(&RequestRoute) -> bool) -> Vec<PendingApproval> {
        let mut rows: Vec<PendingApproval> = self
            .parked
            .request_routes
            .lock()
            .values()
            .filter(|route| keep(route))
            .map(|route| route.pending.clone())
            .collect();
        rows.sort_by_key(|row| row.created_at);
        rows
    }

    /// List recently decided requests for durable audit views.
    pub fn list_recent_decisions(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<super::types::ApprovalAuditEntry>> {
        let limit = limit.clamp(1, 500);
        let mut entries = self
            .run_engine(|engine| async move { engine.history(limit).await })?
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.decided_at));
        entries.truncate(limit);
        Ok(entries)
    }

    /// List undecided requests correlated with a specific flow run (issue
    /// flow-approval-surface, PR2) — lets a dedicated Workflows review
    /// surface fetch just the gates blocking one run instead of filtering
    /// [`Self::list_pending`] client-side.
    pub fn list_pending_for_flow_run(
        &self,
        flow_id: &str,
        run_id: &str,
    ) -> anyhow::Result<Vec<PendingApproval>> {
        Ok(self
            .list_pending()?
            .into_iter()
            .filter(|row| {
                matches!(
                    &row.source_context,
                    Some(ApprovalSourceContext::Flow { flow_id: f, run_id: r, .. })
                        if f == flow_id && r == run_id
                )
            })
            .collect())
    }

    /// Grant "approve always for this flow" trust to `(flow_id, tool_name)`.
    /// Called by the `approval_decide` RPC handler after an
    /// [`ApprovalDecision::ApproveAlwaysForFlow`] decides a flow-origin row —
    /// mirrors the RPC-owns-persistence split documented on
    /// [`Self::decide`] for `ApproveAlwaysForTool`. The engine keeps the grant
    /// durably and approves that exact tool in that exact flow from then on.
    pub fn insert_flow_trust(&self, flow_id: &str, tool_name: &str) -> anyhow::Result<()> {
        let tenant = current_route_tenant();
        let (flow, tool) = (flow_id.to_owned(), tool_name.to_owned());
        self.run_engine(
            |engine| async move { engine.grant_flow_trust(&tenant, &flow, &tool).await },
        )?
        .map_err(|error| anyhow::anyhow!("could not save the flow trust: {error}"))
    }

    /// Whether `(flow_id, tool_name)` currently holds "approve always for
    /// this flow" trust. Exposed for tests and diagnostics.
    pub fn is_flow_tool_trusted(&self, flow_id: &str, tool_name: &str) -> anyhow::Result<bool> {
        Ok(self
            .list_flow_trust(flow_id)?
            .iter()
            .any(|tool| tool == tool_name))
    }

    /// Every `tool_name` currently trusted for `flow_id`, sorted. Consumed by
    /// `flows_approval_manifest` to diff the graph's required permissions
    /// against grants that already exist (re-save asks only for what's new).
    pub fn list_flow_trust(&self, flow_id: &str) -> anyhow::Result<Vec<String>> {
        let tenant = current_route_tenant();
        let flow = flow_id.to_owned();
        self.run_engine(|engine| async move { engine.flow_trust(&tenant, &flow).await })?
            .map_err(|error| anyhow::anyhow!("could not read the flow trust: {error}"))
    }

    /// Revoke flow trust: all grants for `flow_id` when `tool_names` is
    /// `None` (flow deletion cleanup), or only the named grants. Returns the
    /// number of grants removed.
    pub fn delete_flow_trust(
        &self,
        flow_id: &str,
        tool_names: Option<&[String]>,
    ) -> anyhow::Result<usize> {
        let tools = match tool_names {
            Some(names) => names.to_vec(),
            None => self.list_flow_trust(flow_id)?,
        };
        let tenant = current_route_tenant();
        let mut removed = 0;
        for tool in tools {
            let (flow, name, tenant) = (flow_id.to_owned(), tool, tenant.clone());
            let revoked = self
                .run_engine(|engine| async move {
                    engine.revoke_flow_trust(&tenant, &flow, &name).await
                })?
                .map_err(|error| anyhow::anyhow!("could not revoke the flow trust: {error}"))?;
            if revoked {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Return the session id this gate was installed with (used by
    /// RPC handlers for diagnostics).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Clone the instance barrier without retaining the routing-map lock.
    fn request_scope(&self, request_id: &str) -> Option<Arc<super::ApprovalScope>> {
        self.parked
            .request_routes
            .lock()
            .get(request_id)
            .and_then(|route| route.approval_scope.clone())
    }

    /// The request_id of the approval currently parked on `thread_id` by the
    /// ambient tenant (SaaS profile and/or embedded agent), if any. Used by
    /// the web channel to route an inbound yes/no reply to a decision.
    pub fn pending_for_thread(&self, thread_id: &str) -> Option<String> {
        self.pending_for_tenant_thread(&super::gate::current_route_tenant(), thread_id)
    }

    /// The request_id `agent` (`None`: the process) has parked on
    /// `thread_id` outside any SaaS profile, if any.
    pub fn pending_for_agent_thread(&self, agent: Option<&str>, thread_id: &str) -> Option<String> {
        let tenant = crate::core::runtime::Tenant {
            profile: None,
            agent: agent.map(str::to_owned),
        };
        self.pending_for_tenant_thread(&tenant, thread_id)
    }

    /// The request_id `tenant` has parked on `thread_id`, if any.
    pub fn pending_for_tenant_thread(
        &self,
        tenant: &crate::core::runtime::Tenant,
        thread_id: &str,
    ) -> Option<String> {
        self.parked
            .thread_to_request
            .lock()
            .get(&thread_route_key(tenant, thread_id))
            .cloned()
    }

    /// The full pending row parked on `thread_id`, if any.
    ///
    /// [`Self::pending_for_thread`] answers *which* request is parked; this
    /// answers *what it is*, which is what a UI needs to rebuild the approval
    /// card from scratch.
    ///
    /// The card reaches the UI as ONE fire-and-forget socket emit. If that emit
    /// misses — the addressed client's room is empty because the page reloaded,
    /// a rejoining socket was not yet in the thread room, or the bridge dropped
    /// the frame on broadcast lag — nothing re-sends it. The request stays
    /// parked server-side and the user is left with no card and no way to act,
    /// which is the whole failure: durable state delivered as an ephemeral
    /// event, with no way to reconcile the two. This lookup is that
    /// reconciliation, so a socket (re)joining a thread can be handed whatever
    /// is parked on it.
    ///
    /// Reads the routing map twice, so a request decided in between simply
    /// falls out as `None` rather than replaying a card the user has already
    /// answered.
    pub fn parked_request_for_thread(&self, thread_id: &str) -> Option<PendingApproval> {
        let request_id = self.pending_for_thread(thread_id)?;
        let row = self
            .parked
            .request_routes
            .lock()
            .get(&request_id)
            .map(|route| route.pending.clone())?;
        // Re-read the thread route before answering: between the two reads the
        // mapping can be cleared by a caller-bound abandon or overwritten by a
        // replacement turn parking a new approval on the same thread (#4774's
        // scenario). Either leaves this row pending while it is no longer the
        // thread's, and the only caller replays what it gets straight onto the
        // user's screen — so a stale row here is a stale approval card, not a
        // harmless read.
        //
        // Narrowing, not closing: the request itself could still be decided
        // after this check. That is the pre-existing fire-and-forget property of
        // the replay path, and `request_id` idempotency on the client is what
        // covers it (#6212 review).
        self.pending_for_thread(thread_id)
            .is_some_and(|current| current == request_id)
            .then_some(row)
    }

    /// Every approval still parked on `thread_id` by the ambient context's
    /// agent, oldest first.
    ///
    /// [`Self::parked_request_for_thread`] follows the single thread route,
    /// which holds only the newest park. Several async-delegated sub-agents can
    /// park on the same parent thread at once; each keeps its own waiter and
    /// [`RequestRoute`], so replay reads those instead and a rejoining client
    /// can rebuild every card, not only the last one raised.
    pub fn parked_requests_for_thread(&self, thread_id: &str) -> Vec<PendingApproval> {
        let agent = crate::core::runtime::agent_scope::current_agent_id();
        let mut rows: Vec<PendingApproval> = self
            .parked
            .request_routes
            .lock()
            .values()
            .filter(|route| {
                route.thread_id.as_deref() == Some(thread_id)
                    && route.agent_id.as_deref() == agent.as_deref()
            })
            .map(|route| route.pending.clone())
            .collect();
        rows.sort_by_key(|row| row.created_at);
        rows
    }

    /// Whether the parked `request_id` was routed to its chat thread from a
    /// detached task (an async-delegated sub-agent), so the replayed card keeps
    /// the live event's `detached` flag. `false` for an unknown request.
    pub fn request_is_detached(&self, request_id: &str) -> bool {
        self.parked
            .request_routes
            .lock()
            .get(request_id)
            .is_some_and(|route| route.detached)
    }
}

fn validate_decision_tenant(
    dispatch: &crate::core::runtime::Tenant,
    requested_agent: Option<&str>,
    parked: &crate::core::runtime::Tenant,
    request_id: &str,
) -> Result<(), ApprovalError> {
    if dispatch.profile != parked.profile {
        return Err(ApprovalError::WrongTenant {
            request_id: request_id.to_owned(),
        });
    }
    if dispatch.agent.is_some() && dispatch.agent != parked.agent {
        return Err(ApprovalError::WrongAgent {
            request_id: request_id.to_owned(),
        });
    }
    // The legacy process-scoped RPC may omit `agent_id`. Its authenticated
    // tenant is still checked above, and the decision and any durable grant
    // remain bound to this exact parked route. An explicit target must match.
    if requested_agent.is_some_and(|agent| parked.agent.as_deref() != Some(agent)) {
        return Err(ApprovalError::WrongAgent {
            request_id: request_id.to_owned(),
        });
    }
    Ok(())
}
