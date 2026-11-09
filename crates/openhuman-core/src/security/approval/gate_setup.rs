impl ApprovalGate {
    /// Install the process-global gate. Returns the existing gate if
    /// one was already installed (re-install is a no-op so repeated
    /// `bootstrap_core_runtime` calls in tests don't panic).
    ///
    /// Requests left by a previous launch are not touched on install. They
    /// have no live parked future and the engine cannot resume another
    /// instance's request, so they expire on their own and stay in the history.
    pub fn init_global(config: Config, session_id: impl Into<String>) -> Arc<ApprovalGate> {
        let session_id = session_id.into();
        if let Some(existing) = GLOBAL_GATE.get() {
            return existing.clone();
        }
        // TinySecurity owns the default expiry; the host only supplies a
        // shorter per-call bound when one of its surfaces requires it.
        let ttl = None;
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

    /// An isolated gate for tests outside this module. Tests use the same
    /// scripted engine as the gate's unit suite, so no native module or host
    /// approval database is required.
    #[cfg(test)]
    pub(crate) fn for_test(workspace: &std::path::Path) -> Self {
        let config = Config {
            workspace_dir: workspace.to_path_buf(),
            ..Config::default()
        };
        let session = format!("session-{}", uuid::Uuid::new_v4());
        Self::with_engine(
            config,
            session,
            None,
            super::engine_stub::StubEngine::new(),
        )
    }

    /// The engine this build asks: the TinySecurity module, or, in a build
    /// without it, one that refuses every gated call.
    fn engine_for(config: &Config) -> SharedEngine {
        #[cfg(feature = "security-module")]
        {
            Arc::new(super::native::NativeEngine::production(config.clone()))
        }
        #[cfg(not(feature = "security-module"))]
        {
            let _ = config;
            Arc::new(super::engine::UnavailableEngine)
        }
    }

    fn new(config: Config, session_id: String, ttl: Option<Duration>) -> Self {
        let engine = Self::engine_for(&config);
        Self::with_engine(config, session_id, ttl, engine)
    }

    /// A gate over `engine`: how the tests drive the gate's own behaviour
    /// (parking, routing, events) without loading the module.
    pub(crate) fn with_engine(
        config: Config,
        session_id: String,
        ttl: Option<Duration>,
        engine: SharedEngine,
    ) -> Self {
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
            engine,
            parked: Arc::new(Parked::default()),
            audit_failure: AtomicBool::new(false),
        }
    }

    /// Whether a required terminal audit write has failed during this process.
    pub(crate) fn audit_failure_latched(&self) -> bool {
        self.audit_failure.load(Ordering::SeqCst)
    }

    /// Fail closed after any required host or approval terminal audit write
    /// could not be persisted.
    pub(crate) fn latch_audit_failure(&self) {
        self.audit_failure.store(true, Ordering::SeqCst);
    }

    /// The park window the user asks for, if any. In debug builds
    /// `OPENHUMAN_APPROVAL_TTL_SECS` overrides the configured value per intercept
    /// so E2E tests can exercise the timeout path without waiting the module's
    /// full window.
    ///
    /// The override is compiled out of release builds (`#[cfg(debug_assertions)]`):
    /// the shipped product never reads this env var, so a hostile process
    /// environment cannot shorten the supervised-mode approval window. This
    /// mirrors the host-aware discipline of the `OPENHUMAN_APPROVAL_GATE`
    /// kill-switch — neither override can make the gate fail open; the timeout
    /// path always denies. `None` leaves the window to the module, which owns
    /// the default (ten minutes; three for a copilot or sub-agent) and only ever
    /// shortens it for a request.
    fn effective_ttl(&self) -> Option<Duration> {
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
            return Some(ttl);
        }
        self.ttl
    }

    /// The user's "Always allow" tool list. Prefers the agent or process-global
    /// live policy (so a grant made this session is seen immediately) and falls
    /// back to the gate's boot-time config snapshot.
    fn live_auto_approve(&self, ctx: &crate::core::runtime::DispatchCtx) -> Vec<String> {
        if let Some(policy) = crate::security::live_policy::effective_in(ctx) {
            return policy.auto_approve.clone();
        }
        self.config.autonomy.auto_approve.clone()
    }

    /// Whether the user has opted into "auto-approve everything" — a
    /// blanket bypass of the human approval prompt. Mirrors
    /// [`Self::live_auto_approve`]'s live-policy-first, boot-config-
    /// fallback pattern so a toggle made this session (config save + live
    /// policy reload) takes effect on the very next tool call.
    ///
    /// Callers MUST still exclude `AgentTurnOrigin::Unknown` before trusting
    /// this flag — an unlabelled call never reaches the engine. This method only
    /// reports the user's setting; it does not know about origin.
    fn is_auto_approve_all_enabled(&self, ctx: &crate::core::runtime::DispatchCtx) -> bool {
        if let Some(policy) = crate::security::live_policy::effective_in(ctx) {
            return policy.auto_approve_all;
        }
        self.config.autonomy.auto_approve_all
    }

    /// What the user configured for a call under `variant`, read live: the
    /// engine converts a required approval into an audited automatic one only
    /// as far as this allows. A forced call has no shortcut at all.
    fn posture(&self, ctx: &crate::core::runtime::DispatchCtx, variant: Variant) -> Posture {
        if variant == Variant::Forced {
            return Posture {
                require_approval: true,
                ..Posture::default()
            };
        }
        Posture {
            require_approval: !ctx.approvals_disabled(),
            auto_approve_all: self.is_auto_approve_all_enabled(ctx),
        }
    }

    /// Intercept a tool call. Blocks until the user decides or the
    /// TTL elapses (timeout → `Deny`).
    ///
    /// Use [`Self::intercept_audited`] instead when the caller can
    /// also record the *terminal* status of the tool — the audit
    /// trail only carries before-and-after rows when both sides report
    /// in. See #2135.
    pub async fn intercept(
        &self,
        turn: &ApprovalTurnContext,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> GateOutcome {
        // Callers using this entry point never learn the request id, so they
        // cannot report how the call ended: the approval is not reserved for
        // execution, which would leave the reservation open forever.
        let mut _park_bound_elapsed = false;
        self.intercept_audited_inner(
            turn,
            tool_name,
            action_summary,
            args_redacted,
            None,
            &mut _park_bound_elapsed,
            None,
            Mode::forced(false).untracked(),
        )
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
        // As [`Self::intercept`]: no request id comes back, so nothing is
        // reserved for execution.
        self.intercept_audited_inner(
            &ApprovalTurnContext::default(),
            tool_name,
            action_summary,
            args_redacted,
            None,
            &mut _park_bound_elapsed,
            None,
            Mode::forced(true).untracked(),
        )
        .await
        .0
    }

    /// [`Self::intercept_forced`] that also hands back the request id of an
    /// allowed call, so the caller can [`record_execution`](Self::record_execution)
    /// how the approved action ended.
    pub async fn intercept_forced_audited(
        &self,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> (GateOutcome, Option<String>) {
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
            Mode::forced(true),
        )
        .await
    }

    /// Audited variant of [`Self::intercept`].
    ///
    /// Returns `(outcome, Some(request_id))` when the call was
    /// allowed through the engine — pass the id back to
    /// [`Self::record_execution`] once the tool finishes so the audit row
    /// carries both the approval and the terminal status (issue #2135).
    ///
    /// Returns `(outcome, None)` when the call was allowed by the origin's own
    /// trust (no approval was asked) OR when it was denied. In either case there
    /// is nothing to record afterward, so the caller can pattern-match
    /// `(GateOutcome::Allow, Some(id))` to decide whether to invoke
    /// `record_execution`.
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
    /// and the persisted request can correlate back to the exact `tool_call`
    /// timeline row instead of matching on tool name alone.
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
            Mode::forced(false),
        )
        .await
    }

    /// Like [`Self::intercept_audited`] but the caller may cap how long the
    /// gate parks (issue #4756).
    ///
    /// When `park_bound` is `Some` and shorter than the gate's own effective
    /// TTL and it elapses before a decision arrives, the gate abandons the park
    /// in a **cancellation-safe** way — it evicts the in-memory
    /// waiter and clears the thread routing mapping (so a later chat reply
    /// is not mis-routed to this now-abandoned request) but deliberately LEAVES
    /// the request open, so a later human card-click can still resolve it in the
    /// store — and returns `None`. This is why callers must bound
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
                Mode::forced(false),
            )
            .await;
        if park_bound_elapsed {
            None
        } else {
            Some(resolved)
        }
    }
}
