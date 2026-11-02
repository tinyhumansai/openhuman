use super::*;

impl Default for OpenHumanRunContext {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenHumanRunContext {
    /// Builds an unbound context. Entry points set only the values their turn
    /// actually owns; `None` is an explicit absence, not an ambient fallback.
    pub fn new() -> Self {
        Self {
            model_budget: None,
            dispatch_admission: None,
            response_shape: None,
            max_spawn_depth: None,
            origin: None,
            approval: Default::default(),
            progress: None,
            stop_hooks: Vec::new(),
            goal_suppressed: false,
            tool_hooks: Vec::new(),
            tool_call_limit: None,
            untrusted_input_turn: false,
            turn_deadline: None,
            parent: None,
            prepared_context_sources: Arc::new(Vec::new()),
            memory_identity: None,
            core: None,
            hosted_base: None,
            transcripts: None,
            file_state_agent_id: None,
            tool_result_artifact_index: None,
            headless_descendant_settlement: false,
            headless_descendant_owner: None,
            attachment_placeholders: Arc::new(Vec::new()),
            native_staged_parts: Vec::new(),
            native_staged_lease: None,
            native_message_id: None,
            dispatch: None,
            task_recency_window: None,
            command_environment: None,
            autonomous_iter_cap: None,
            sandbox_mode: None,
            spawn_depth: 0,
            subagent_usage: Arc::new(Mutex::new(Vec::new())),
            parent_subagent_usage: None,
            resolved_route: Arc::new(Mutex::new(None)),
            cancellation: tinyagents_harness::cancel::CancellationToken::default(),
            thread_id: None,
            root_run_id: None,
            workspace: None,
            tool_outcomes: None,
            tool_policy: None,
            current_tools: None,
            current_synthesized_tools: None,
            recorded_tool_names: Arc::new(std::collections::HashSet::new()),
            deferred_tool_names: Arc::new(std::collections::HashSet::new()),
            tool_rules: None,
            context_middleware: None,
            session_sidecar: Arc::new(Mutex::new(SessionTurnSidecar::default())),
            required_output: None,
            tool_dialect: tinyagents_harness::config::ToolDispatcher::Auto,
            memory_turn: None,
            shell_processes: crate::sandbox::shell_processes::ShellProcesses::new(),
        }
    }

    /// Returns the registry owner for a spawn. Headless turns retain one
    /// root-scoped owner across native child thread identities so settlement
    /// can see the complete descendant tree. Ordinary chat remains scoped to
    /// the immediate parent session and thread.
    pub(crate) fn subagent_owner_scope(
        &self,
        workspace: &std::path::Path,
        parent_session: &str,
        thread_id: Option<&str>,
    ) -> String {
        if self.headless_descendant_settlement {
            if let Some(owner) = &self.headless_descendant_owner {
                return owner.clone();
            }
        }
        crate::agent::tinyagents::tool_registration::subagent_owner_key(
            workspace,
            parent_session,
            thread_id,
        )
    }

    /// Pins the harness tool dialect to the one the session prompt speaks.
    #[must_use]
    pub(crate) fn with_tool_dialect(
        mut self,
        dialect: tinyagents_harness::config::ToolDispatcher,
    ) -> Self {
        self.tool_dialect = dialect;
        self
    }

    /// Binds the context this run acts under, held weakly so a run kept past its turn never pins the tenant.
    #[must_use]
    pub fn with_core(mut self, core: Option<Arc<crate::core::runtime::CoreContext>>) -> Self {
        self.core = core.as_ref().map(Arc::downgrade);
        self
    }

    /// The dispatch context this run's readers use: its own `core`, else the
    /// calling task's (a run assembled by a caller that did not bind one).
    pub fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref())
    }

    /// The connected-identities prompt section, loaded asynchronously under this run's context.
    pub(crate) async fn connected_identities_md(&self) -> String {
        crate::integrations::render_connected_identities_async(&self.dispatch_ctx()).await
    }

    /// Sets the direct TinyTools workspace descriptor for this run.
    pub fn with_workspace(mut self, workspace: tinytools::WorkspaceDescriptor) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Creates (once) this host turn's durable-unique root config.
    ///
    /// The returned root id is stable across the session/runtime and hosted
    /// harness boundaries for this one actual turn. Callers creating a new
    /// root must call this once rather than construct a config from a repeated
    /// literal; callers creating children must use [`direct_subagent_child`].
    pub(crate) fn root_run_config(&mut self, kind: &str) -> tinyagents_harness::context::RunConfig {
        let run_id = self
            .root_run_id
            .get_or_insert_with(|| fresh_root_run_config(kind).run_id.as_str().to_owned())
            .clone();
        tinyagents_harness::context::RunConfig::new(run_id)
    }

    /// Binds a direct runner to its explicit parent without exposing the
    /// child-ledger link reserved for recursive [`Self::child`] calls.
    pub(crate) fn with_parent(mut self, parent: ParentExecutionContext) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Installs this turn's parent snapshot, binding the run's live progress
    /// sink to it.
    ///
    /// The snapshot is built from a prelude that captured `on_progress` when
    /// the runtime session was first created. A web-chat turn checks out a
    /// cached session — so that build is a no-op — and only afterwards calls
    /// `set_on_progress`, leaving the prelude's copy `None` for the rest of the
    /// session's life. Sub-agent spawn and completion are the only progress
    /// events that ride this parent sink instead of the harness event
    /// projection, so a stale `None` drops them outright: no `subagent_spawned`
    /// socket event, no run-ledger row, and a "Background tasks" panel that
    /// reads "none running" while sub-agents are working. The run context's own
    /// sink is the live one (it is what `driver.rs` hands the turn graph), so
    /// prefer it and keep the snapshot as the fallback.
    ///
    /// Returns the installed parent so a caller that must launch a sub-agent
    /// *before* the rest of the turn is assembled — `inject_triggered_memory_agent_context`
    /// is the one such caller — hands it the bound context rather than the
    /// stale snapshot it started from.
    pub(crate) fn attach_parent(
        &mut self,
        mut parent: ParentExecutionContext,
    ) -> &ParentExecutionContext {
        parent.on_progress = self.progress.clone().or(parent.on_progress);
        self.parent.insert(parent)
    }

    /// The callbacks and caps this turn's harness assembly installs.
    pub(crate) fn harness_host_policy(&self) -> HarnessHostPolicy {
        HarnessHostPolicy {
            stop_hooks: self.stop_hooks.clone(),
            tool_hooks: self.tool_hooks.clone(),
            tool_call_cap: self.tool_call_limit,
        }
    }

    /// The real tool invocation budget for a turn of `max_iterations`: the
    /// product default, narrowed by the host's explicit cap.
    pub(crate) fn tool_call_limit_for(&self, max_iterations: usize) -> usize {
        crate::agent::tinyagents::turn_policy::tool_call_limit(max_iterations, self.tool_call_limit)
    }

    /// Applies the host environment an entry point handed to the session:
    /// shared budget, delegation depth, cancellation, deadline and hooks.
    #[must_use]
    pub(crate) fn with_host_env(mut self, env: &crate::agent::turn_env::HostTurnEnv) -> Self {
        self.model_budget = env.model_budget.clone().or(self.model_budget);
        self.dispatch_admission = env.dispatch_admission.clone().or(self.dispatch_admission);
        self.max_spawn_depth = env.spawn_depth_limit.or(self.max_spawn_depth);
        if let Some(cancellation) = &env.cancellation {
            self.cancellation = cancellation.clone();
        }
        self.turn_deadline = env.turn_deadline.or(self.turn_deadline);
        self.tool_call_limit = env.tool_call_limit.or(self.tool_call_limit);
        self.command_environment = env.command_environment.clone().or(self.command_environment);
        self.memory_identity = env.memory_identity.clone().or(self.memory_identity);
        self.core = env.core.clone().or(self.core);
        self.native_staged_parts = env.native_staged_parts.clone();
        self.native_staged_lease = env.native_staged_lease.clone();
        self.native_message_id = env.native_message_id.clone();
        self.stop_hooks
            .extend(env.hooks.stop_hooks().iter().cloned());
        self.tool_hooks
            .extend(env.hooks.tool_hooks().iter().cloned());
        self
    }

    /// Sets the same cancellation token on this context and its TinyAgents run.
    pub fn with_cancellation(
        mut self,
        cancellation: tinyagents_harness::cancel::CancellationToken,
    ) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Builds a child context that inherits authority, cancellation and shared
    /// workspace policy while keeping route observation and usage isolated.
    pub fn child(&self) -> Self {
        let mut child = self.clone();
        child.spawn_depth = self.spawn_depth.saturating_add(1);
        child.file_state_agent_id = None;
        child.parent_subagent_usage = Some(self.subagent_usage.clone());
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.resolved_route = Arc::new(Mutex::new(None));
        child.memory_turn = None;
        child.native_staged_parts.clear();
        child.native_staged_lease = None;
        child.native_message_id = None;
        child.response_shape = None;
        child.shell_processes = crate::sandbox::shell_processes::ShellProcesses::new();
        child
    }

    /// Builds the explicit carrier for detached background work.
    ///
    /// A detached task keeps the authority, thread and workspace descriptor it
    /// needs to execute and deliver its own result, but it is no longer part of
    /// the originating turn's dispatch timer or subagent usage roll-up. It
    /// still inherits the shared monetary budget and physical-dispatch policy,
    /// and it does not observe cancellation that only stopped the interactive
    /// turn.
    pub fn detached_child(&self) -> Self {
        let mut child = self.child();
        // The chat surface belongs to the turn that started it. A detached
        // child's parks route through the origin and are marked detached, so
        // their cards survive the parent turn's end (#5499).
        child.approval = Default::default();
        child.dispatch = None;
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.parent_subagent_usage = None;
        // Detached work has its own terminal boundary; its typed refusal must
        // never be mistaken for the interactive parent's failure.
        child.session_sidecar = Arc::new(Mutex::new(SessionTurnSidecar::default()));
        child.cancellation = tinyagents_harness::cancel::CancellationToken::new();
        child.headless_descendant_settlement = false;
        child.headless_descendant_owner = None;
        child
    }

    /// Converts this host context into TinyAgents' canonical run context.
    ///
    /// The canonical context receives the exact same cancellation and direct
    /// `tinytools::WorkspaceDescriptor`; no type alias, re-export or task-local
    /// bridge is involved.
    pub fn into_tinyagents(
        mut self,
        mut config: tinyagents_harness::context::RunConfig,
    ) -> tinyagents_harness::context::RunContext<Self> {
        // Imported transport facts and explicit child context must survive entry
        // into the native driver, which scopes the depth recorded in RunConfig.
        let depth = config
            .depth()
            .max(self.spawn_depth)
            .max(tinyagents_harness::context::current_run_depth().map_or(0, |facts| facts.depth));
        self.spawn_depth = depth;
        config = config.with_depth(depth);
        if config.thread_id.is_none() {
            if let Some(thread_id) = self.thread_id.as_deref() {
                config = config.with_thread(thread_id);
            }
        }
        let cancellation = self.cancellation.clone();
        let workspace = self.workspace.clone();
        let run_ext = OpenHumanRunExt::from_run_context(&self);
        let host_operations = Arc::new(HostOperationContext {
            origin: self.origin.clone(),
            approval: self.approval.clone(),
            sandbox_mode: self.sandbox_mode,
            parent_agent_id: self
                .parent
                .as_ref()
                .map(|parent| parent.agent_definition_id.clone()),
            memory_identity: self.memory_identity.clone(),
            memory_turn: self.memory_turn.clone(),
            core: self.core.clone(),
        });
        let context = tinyagents_harness::context::RunContext::new(config, self)
            .with_cancellation(cancellation)
            .with_state_view(host_operations)
            .with_host(run_ext);
        match workspace {
            Some(workspace) => context.with_workspace(workspace),
            None => context,
        }
    }

    /// Records a child usage entry without relying on a task-local collector.
    pub fn append_subagent_usage(&self, entry: SubagentUsageEntry) {
        self.subagent_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Record a completed child on the immediate parent's ledger. This makes
    /// completed children visible at the root without leaking siblings into one
    /// another's in-flight ledger. Direct/root callers retain their own entry.
    pub fn record_completed_subagent_usage(&self, entry: SubagentUsageEntry) {
        let ledger = self
            .parent_subagent_usage
            .as_ref()
            .unwrap_or(&self.subagent_usage);
        ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Snapshot child totals after the child has returned. A snapshot, rather
    /// than a task-local drain, preserves totals even when sibling futures are
    /// cancelled or one child fails after another has completed.
    pub fn subagent_usage_entries(&self) -> Vec<SubagentUsageEntry> {
        self.subagent_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Promote already-completed descendants when this run cannot produce its
    /// own terminal outcome. This is intentionally a snapshot: callers invoke
    /// it only from one terminal finalizer, so every completed descendant is
    /// promoted once without exposing a shared, in-flight sibling ledger.
    pub fn promote_completed_descendant_usage(&self) {
        for entry in self.subagent_usage_entries() {
            self.record_completed_subagent_usage(entry);
        }
    }

    /// Resolves whether a file-state scope has been assigned to this context.
    /// Keeping the conversion here makes file-state callers use the explicit
    /// context instead of discovering an ambient identifier.
    pub fn file_state_agent_id(&self) -> Option<String> {
        self.file_state_agent_id.clone()
    }
}
