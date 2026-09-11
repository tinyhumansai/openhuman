impl ApprovalGate {
    /// Write the *terminal* status of a tool call onto its approval
    /// audit row — see [`store::record_execution`] for semantics.
    ///
    /// Logs (but does not propagate) write errors: the tool has
    /// already run, so audit-log loss should never bubble up as a
    /// tool execution failure to the agent. If durable audit storage
    /// is required for compliance, callers wire it via a stronger
    /// guarantee than this best-effort hook.
    pub fn record_execution(
        &self,
        request_id: &str,
        outcome: ExecutionOutcome,
        error: Option<&str>,
    ) {
        match store::record_execution(&self.config, request_id, outcome, error) {
            Ok(true) => tracing::debug!(
                request_id = %request_id,
                outcome = outcome.as_str(),
                "[approval::gate] recorded terminal execution"
            ),
            Ok(false) => tracing::warn!(
                request_id = %request_id,
                outcome = outcome.as_str(),
                "[approval::gate] record_execution found no matching decided row"
            ),
            Err(err) => tracing::error!(
                request_id = %request_id,
                outcome = outcome.as_str(),
                error = %err,
                "[approval::gate] record_execution write failed"
            ),
        }
    }

    /// Apply a user decision. Returns the now-decided
    /// [`PendingApproval`] row when one was found.
    pub fn decide(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> anyhow::Result<Option<PendingApproval>> {
        let decided = store::decide(&self.config, request_id, decision)?;
        if let Some(row) = &decided {
            // `ApproveAlwaysForTool` persistence (append to `autonomy.auto_approve`
            // + reload the live policy) is handled by the `approval_decide` RPC
            // handler, which is async and owns the config save+reload path. The
            // gate only resolves the parked future and emits the audit event.
            if let Some(tx) = self.take_waiter(request_id) {
                let _ = tx.send(decision);
            }
            BUS.publish(DomainEvent::ApprovalDecided {
                request_id: row.request_id.clone(),
                tool_name: row.tool_name.clone(),
                decision: decision.as_str().to_string(),
            });
        }
        Ok(decided)
    }

    /// Classify a [`Self::decide`] miss — i.e. when `decide` returned
    /// `Ok(None)` because its conditional `UPDATE ... WHERE decided_at IS NULL`
    /// matched 0 rows. Two very different states collapse into that `None`:
    ///
    /// - [`DecideMiss::AlreadyResolved`] — the row exists but was **already
    ///   decided, lazily expired (denied), or superseded**. This is the benign
    ///   double-tap / two-operator / expiry-while-live race the inline-approvals
    ///   design spec classifies as benign (TAURI-RUST-5EH).
    /// - [`DecideMiss::NeverRegistered`] — no row was ever persisted for this
    ///   request_id. That is a genuine lost registration (a core restart dropped
    ///   the parked future before persisting, or a stray id) and must stay a
    ///   Sentry signal.
    ///
    /// We disambiguate by consulting [`store::get_decision`], which returns a
    /// decision only when `decided_at IS NOT NULL` — exactly the already-resolved
    /// case (expiry writes a `Deny` decision, so expired rows report here too).
    /// A `decide` miss can't be an undecided-but-present row: that row would have
    /// matched the `UPDATE`. If the lookup itself errors we conservatively keep
    /// the event visible (`NeverRegistered`) rather than silently demoting.
    pub fn classify_decide_miss(&self, request_id: &str) -> DecideMiss {
        match store::get_decision(&self.config, request_id) {
            Ok(Some(_)) => DecideMiss::AlreadyResolved,
            Ok(None) => DecideMiss::NeverRegistered,
            Err(err) => {
                tracing::warn!(
                    request_id = %request_id,
                    error = %err,
                    "[approval::gate] classify_decide_miss: get_decision failed; treating as never-registered (keep visible)"
                );
                DecideMiss::NeverRegistered
            }
        }
    }

    /// List all undecided rows, including orphans from prior launches.
    /// Orphan rows have no live parked future so a `decide` on them
    /// updates the DB but cannot resume an action — see [`store::list_pending`].
    pub fn list_pending(&self) -> anyhow::Result<Vec<PendingApproval>> {
        store::list_pending(&self.config)
    }

    /// List recently decided rows for durable audit views.
    pub fn list_recent_decisions(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<super::types::ApprovalAuditEntry>> {
        store::list_recent_decisions(&self.config, limit)
    }

    /// List undecided rows correlated with a specific flow run (issue
    /// flow-approval-surface, PR2) — lets a dedicated Workflows review
    /// surface fetch just the gates blocking one run instead of filtering
    /// [`Self::list_pending`] client-side.
    pub fn list_pending_for_flow_run(
        &self,
        flow_id: &str,
        run_id: &str,
    ) -> anyhow::Result<Vec<PendingApproval>> {
        store::list_pending_for_flow_run(&self.config, flow_id, run_id)
    }

    /// Grant "approve always for this flow" trust to `(flow_id, tool_name)`.
    /// Called by the `approval_decide` RPC handler after an
    /// [`ApprovalDecision::ApproveAlwaysForFlow`] decides a flow-origin row —
    /// mirrors the RPC-owns-persistence split documented on
    /// [`Self::decide`] for `ApproveAlwaysForTool`.
    pub fn insert_flow_trust(&self, flow_id: &str, tool_name: &str) -> anyhow::Result<()> {
        store::insert_flow_trust(&self.config, flow_id, tool_name)
    }

    /// Whether `(flow_id, tool_name)` currently holds "approve always for
    /// this flow" trust. Exposed for tests and diagnostics; `intercept_audited`
    /// consults [`store::is_flow_tool_trusted`] directly.
    pub fn is_flow_tool_trusted(&self, flow_id: &str, tool_name: &str) -> anyhow::Result<bool> {
        store::is_flow_tool_trusted(&self.config, flow_id, tool_name)
    }

    /// Every `tool_name` currently trusted for `flow_id`, sorted. Consumed by
    /// `flows_approval_manifest` to diff the graph's required permissions
    /// against grants that already exist (re-save asks only for what's new).
    pub fn list_flow_trust(&self, flow_id: &str) -> anyhow::Result<Vec<String>> {
        store::list_flow_trust(&self.config, flow_id)
    }

    /// Revoke flow trust: all grants for `flow_id` when `tool_names` is
    /// `None` (flow deletion cleanup), or only the named grants. Returns the
    /// number of rows removed.
    pub fn delete_flow_trust(
        &self,
        flow_id: &str,
        tool_names: Option<&[String]>,
    ) -> anyhow::Result<usize> {
        store::delete_flow_trust(&self.config, flow_id, tool_names)
    }

    /// Write the durable audit record for one save-time pre-authorization
    /// grant (a born-decided `approve_always_for_flow` row) so blanket
    /// grants stay inspectable in Settings → Approval history.
    pub fn record_flow_preauthorization(
        &self,
        flow_id: &str,
        tool_name: &str,
    ) -> anyhow::Result<()> {
        store::record_flow_preauthorization(&self.config, flow_id, tool_name, &self.session_id)
    }

    /// Return the session id this gate was installed with (used by
    /// RPC handlers for diagnostics).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn take_waiter(&self, request_id: &str) -> Option<oneshot::Sender<ApprovalDecision>> {
        let mut waiters = self.waiters.lock();
        waiters.remove(request_id)
    }

    fn evict_waiter(&self, request_id: &str) {
        let mut waiters = self.waiters.lock();
        waiters.remove(request_id);
    }

    /// The request_id of the approval currently parked on `thread_id`, if any.
    /// Used by the web channel to route an inbound yes/no reply to a decision.
    pub fn pending_for_thread(&self, thread_id: &str) -> Option<String> {
        self.thread_to_request.lock().get(thread_id).cloned()
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
    /// Reads the routing map first and the durable rows second, so a request
    /// decided between the two reads simply falls out as `None` rather than
    /// replaying a card the user has already answered.
    pub fn parked_request_for_thread(&self, thread_id: &str) -> Option<PendingApproval> {
        let request_id = self.pending_for_thread(thread_id)?;
        let row = self
            .list_pending()
            .ok()?
            .into_iter()
            .find(|row| row.request_id == request_id)?;
        // Re-read the route before answering. `list_pending` is a store read and
        // runs without the route lock held, so between the two the mapping can
        // be cleared by a caller-bound abandon or overwritten by a replacement
        // turn parking a new approval on the same thread (#4774's scenario).
        // Either leaves this row pending while it is no longer the thread's, and
        // the only caller replays what it gets straight onto the user's screen —
        // so a stale row here is a stale approval card, not a harmless read.
        //
        // Narrowing, not closing: the row itself could still be decided after
        // this check. That is the pre-existing fire-and-forget property of the
        // replay path, and `request_id` idempotency on the client is what covers
        // it; this only stops the lookup from *starting* with a row it can
        // already tell is not the thread's (#6212 review).
        self.pending_for_thread(thread_id)
            .is_some_and(|current| current == request_id)
            .then_some(row)
    }

    /// Drop the thread → request mapping when it still belongs to this request.
    fn clear_thread(&self, thread_id: &Option<String>, request_id: &str) {
        if let Some(t) = thread_id {
            self.clear_thread_route_if_owned(t, request_id);
        }
    }

    /// Drop the thread → request mapping **only if** it still points at
    /// `request_id`. Used by [`WaiterGuard::drop`] on external teardown, where a
    /// replacement turn may have already parked a new approval on the same
    /// thread and overwritten the entry; clearing unconditionally would delete
    /// the *new* request's routing (#4774).
    fn clear_thread_route_if_owned(&self, thread_id: &str, request_id: &str) {
        let mut map = self.thread_to_request.lock();
        if map.get(thread_id).is_some_and(|rid| rid == request_id) {
            map.remove(thread_id);
        }
    }
}
