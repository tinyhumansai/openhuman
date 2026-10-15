impl ApprovalGate {
    /// Install the process-global gate. Returns the existing gate if
    /// one was already installed (re-install is a no-op so repeated
    /// `bootstrap_core_runtime` calls in tests don't panic).
    ///
    /// Rows from prior launches are intentionally NOT purged on
    /// install — the issue #1339 acceptance criterion requires they
    /// survive restart so the UI can show / dismiss them. Orphan
    /// rows have no live parked future, so a `decide` is a DB-only
    /// audit update; no side effect can fire across processes.
    pub fn init_global(config: Config, session_id: impl Into<String>) -> Arc<ApprovalGate> {
        let session_id = session_id.into();
        if let Some(existing) = GLOBAL_GATE.get() {
            return existing.clone();
        }
        let ttl = config
            .security
            .approval
            .ttl_secs
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_APPROVAL_TTL);
        let gate = Arc::new(ApprovalGate::new(config, session_id, ttl));
        let _ = GLOBAL_GATE.set(gate.clone());
        GLOBAL_GATE.get().cloned().unwrap_or(gate)
    }

    /// Returns the global gate when installed; tools and harness
    /// branches that don't care about supervised mode treat `None`
    /// as "no gating".
    pub fn try_global() -> Option<Arc<ApprovalGate>> {
        GLOBAL_GATE.get().cloned()
    }

    fn new(config: Config, session_id: String, ttl: Duration) -> Self {
        // Regression guard: the gate's session_id must be the per-launch
        // UUID minted by `bootstrap_core_runtime` (shape:
        // `session-<uuid>`). Any other shape risks re-introducing the
        // credential leak that was fixed by switching off the RPC bearer
        // — fail loudly in debug builds the moment a caller wires up a
        // raw token (or any other ad-hoc string).
        #[cfg(debug_assertions)]
        debug_assert!(
            session_id.starts_with("session-"),
            "ApprovalGate session_id must be a per-launch UUID prefix, not a credential",
        );
        Self {
            config,
            session_id,
            ttl,
            waiters: Mutex::new(HashMap::new()),
            thread_to_request: Mutex::new(HashMap::new()),
            request_routes: Mutex::new(HashMap::new()),
        }
    }

    /// TTL for parking an approval. In debug builds `OPENHUMAN_APPROVAL_TTL_SECS`
    /// overrides the boot-time default per intercept so E2E tests can exercise
    /// the timeout path without waiting the full `DEFAULT_APPROVAL_TTL`.
    ///
    /// The override is compiled out of release builds (`#[cfg(debug_assertions)]`):
    /// the shipped product never reads this env var, so a hostile process
    /// environment cannot shorten the supervised-mode approval window. This
    /// mirrors the host-aware discipline of the `OPENHUMAN_APPROVAL_GATE`
    /// kill-switch — neither override can make the gate fail open; the timeout
    /// path always denies.
    fn effective_ttl(&self) -> Duration {
        #[cfg(debug_assertions)]
        if let Some(ttl) = std::env::var("OPENHUMAN_APPROVAL_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
        {
            tracing::debug!(
                ttl_secs = ttl.as_secs(),
                "[approval::gate] TTL env override active (debug build)"
            );
            return ttl;
        }
        self.ttl
    }

    /// Resolve the actual park duration from the gate's own `effective_ttl`
    /// plus the `copilot_stream` clamp ([`COPILOT_APPROVAL_TTL`]) and the
    /// sub-agent clamp ([`SUBAGENT_APPROVAL_TTL`]) when those apply. A clamp only ever *shortens* the park — it can never
    /// extend `effective_ttl` past what the gate itself allows (e.g. a debug
    /// env override). Split out (rather than inlined at the call site) so it is
    /// unit testable without needing to actually park a future.
    fn resolve_park_ttl(effective_ttl: Duration, copilot_stream: bool, subagent: bool) -> Duration {
        let mut ttl = effective_ttl;
        if copilot_stream {
            ttl = ttl.min(COPILOT_APPROVAL_TTL);
        }
        if subagent {
            ttl = ttl.min(SUBAGENT_APPROVAL_TTL);
        }
        ttl
    }

    /// Whether `tool_name` is on the user's "Always allow" list. Prefers the agent or
    /// process-global live policy (so a grant made this session is seen
    /// immediately) and falls back to the gate's boot-time config snapshot.
    fn tool_is_auto_approved(&self, tool_name: &str) -> bool {
        if let Some(policy) = crate::security::live_policy::effective() {
            return policy.auto_approve.iter().any(|t| t == tool_name);
        }
        self.config
            .autonomy
            .auto_approve
            .iter()
            .any(|t| t == tool_name)
    }

    /// Whether the user has opted into "auto-approve everything" — a
    /// blanket bypass of the human approval prompt. Mirrors
    /// [`Self::tool_is_auto_approved`]'s live-policy-first, boot-config-
    /// fallback pattern so a toggle made this session (config save + live
    /// policy reload) takes effect on the very next tool call.
    ///
    /// Callers MUST still exclude `AgentTurnOrigin::Unknown` before trusting
    /// this flag — see the `matches!` guard at the call site below. This method only reports the
    /// user's setting; it does not know about origin.
    fn is_auto_approve_all_enabled(&self) -> bool {
        if let Some(policy) = crate::security::live_policy::effective() {
            return policy.auto_approve_all;
        }
        self.config.autonomy.auto_approve_all
    }

    /// Append an audit row for a call the gate let through without a prompt.
    /// Best effort: an audit failure is logged, never turned into a denial of
    /// a call the policy already allowed.
    pub(super) fn audit_unprompted_allow(
        &self,
        tool_name: &str,
        policy_decision: &str,
        origin_class: &str,
    ) {
        let audit = match crate::security::get_or_create_workspace_audit_logger(
            self.config.security.audit.clone(),
            self.config.workspace_dir.clone(),
        ) {
            Ok(audit) => audit,
            Err(error) => {
                tracing::warn!(%error, "[approval::gate] audit logger unavailable");
                return;
            }
        };
        let event =
            crate::security::AuditEvent::new(crate::security::AuditEventType::PolicyDecision)
                .with_actor(origin_class.to_string(), None, None)
                .with_action(
                    tool_name.to_string(),
                    "external_effect".to_string(),
                    true,
                    true,
                )
                .with_policy_decision(policy_decision);
        if let Err(error) = audit.log(&event) {
            tracing::warn!(%error, policy_decision, "[approval::gate] audit write failed");
        }
    }

    /// Intercept a tool call. Blocks until the user decides or the
    /// TTL elapses (timeout → `Deny`).
    ///
    /// Use [`Self::intercept_audited`] instead when the caller can
    /// also record the *terminal* status of the tool — the audit
    /// trail in `pending_approvals` only carries before-and-after
    /// rows when both sides report in. See #2135.
    pub async fn intercept(
        &self,
        turn: &ApprovalTurnContext,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> GateOutcome {
        // Drop the request_id; callers using the legacy entry point
        // don't record execution.
        self.intercept_audited(turn, tool_name, action_summary, args_redacted)
            .await
            .0
    }

    /// Require an explicit, one-time decision for this exact action. This
    /// entry point ignores all ordinary auto-approval and origin shortcuts.
    /// A turn without a routable WebChat origin fails closed.
    pub async fn intercept_forced(
        &self,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> GateOutcome {
        let mut _park_bound_elapsed = false;
        // A forced approval routes from the validated WebChat origin alone and
        // never reads a chat context, a copilot flag or a flow run, so it takes
        // none.
        self.intercept_audited_inner(
            &ApprovalTurnContext::default(),
            tool_name,
            action_summary,
            args_redacted,
            None,
            &mut _park_bound_elapsed,
            None,
            true,
        )
        .await
        .0
    }

    /// Audited variant of [`Self::intercept`].
    ///
    /// Returns `(outcome, Some(request_id))` when the call was
    /// allowed AND a `pending_approvals` row was persisted — pass
    /// the id back to [`Self::record_execution`] once the tool
    /// finishes so the audit row carries both the approval and the
    /// terminal status (issue #2135).
    ///
    /// Returns `(outcome, None)` when no DB row was created (session
    /// allowlist shortcut) OR when the call was denied. In either
    /// case there is nothing to record afterward, so the caller can
    /// pattern-match `(GateOutcome::Allow, Some(id))` to decide
    /// whether to invoke `record_execution`.
    pub async fn intercept_audited(
        &self,
        turn: &ApprovalTurnContext,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> (GateOutcome, Option<String>) {
        self.intercept_audited_for_call(turn, tool_name, action_summary, args_redacted, None)
            .await
    }

    /// Like [`Self::intercept_audited`], but threads the gated tool call's
    /// provider-assigned call id through so `ApprovalRequested`/`ApprovalDecided`
    /// and the persisted `pending_approvals` row can correlate back to the
    /// exact `tool_call` timeline row instead of matching on tool name alone.
    /// `None` for callers with no tracked call id (a legacy path, or a call
    /// not driven through the tinyagents harness).
    pub async fn intercept_audited_for_call(
        &self,
        turn: &ApprovalTurnContext,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
        tool_call_id: Option<&str>,
    ) -> (GateOutcome, Option<String>) {
        // No caller-supplied park bound: identical behavior to before. With
        // `park_bound = None` the inner never takes the caller-bound abandon
        // path, so the out-flag stays `false` and is discarded here.
        let mut _park_bound_elapsed = false;
        self.intercept_audited_inner(
            turn,
            tool_name,
            action_summary,
            args_redacted,
            None,
            &mut _park_bound_elapsed,
            tool_call_id,
            false,
        )
        .await
    }

    /// Like [`Self::intercept_audited`] but the caller may cap how long the
    /// gate parks (issue #4756).
    ///
    /// When `park_bound` is `Some` and shorter than the gate's own effective
    /// TTL and it elapses before a decision arrives, the gate abandons the park
    /// in a **cancellation-safe** way — it evicts the in-memory waiter and
    /// clears the thread routing mapping (so a later chat reply
    /// is not mis-routed to this now-abandoned request) but deliberately LEAVES
    /// the `pending_approvals` row open, so a later human card-click can still
    /// resolve it in the DB — and returns `None`. This is why callers must bound
    /// the park through the gate rather than racing an outer
    /// `tokio::time::timeout` against [`Self::intercept_audited`]: dropping the
    /// parked future would skip that cleanup and orphan the waiter + routing
    /// mappings (chatgpt-codex review on #4756).
    ///
    /// A `None` bound (or one `>=` the effective TTL) behaves exactly like
    /// [`Self::intercept_audited`] and always returns `Some`.
    pub async fn intercept_audited_bounded(
        &self,
        turn: &ApprovalTurnContext,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
        park_bound: Option<Duration>,
    ) -> Option<(GateOutcome, Option<String>)> {
        let mut park_bound_elapsed = false;
        let resolved = self
            .intercept_audited_inner(
                turn,
                tool_name,
                action_summary,
                args_redacted,
                park_bound,
                &mut park_bound_elapsed,
                None,
                false,
            )
            .await;
        if park_bound_elapsed {
            None
        } else {
            Some(resolved)
        }
    }
}
