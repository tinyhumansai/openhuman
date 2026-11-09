use super::*;

impl OpenHumanSessionHost {
    pub(crate) fn enable_headless_descendant_settlement(&mut self) {
        self.settle_headless_descendants = true;
    }

    /// Restricts integration task-fetch tools to tasks touched within `window`
    /// on every later turn of this agent; `None` clears the restriction.
    pub fn set_task_recency_window(&mut self, window: Option<std::time::Duration>) {
        self.task_recency_window = window;
    }

    /// Hands the host environment of the turns this session runs (shared
    /// model budget, cancellation, deadline, hooks, tool-belt authority).
    ///
    /// Call it after building the session and before its first turn: the
    /// completed-turn callbacks join the learning sink's set and the tool belt
    /// authority is read when the runtime session is first created.
    pub fn set_host_env(&mut self, env: crate::agent::turn_env::HostTurnEnv) {
        for hook in env.hooks.learning_hooks() {
            if !self
                .post_turn_hooks
                .iter()
                .any(|existing| Arc::ptr_eq(existing, hook))
            {
                self.post_turn_hooks.push(Arc::clone(hook));
            }
        }
        tracing::debug!(
            env = ?env,
            "[session_host] host turn env installed"
        );
        self.host_env = env;
    }

    /// The context this session's turns run under: the one its host
    /// environment carries, else the calling task's.
    pub(crate) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.host_env.core.as_ref())
    }

    /// The host environment installed with [`Self::set_host_env`].
    pub fn host_env(&self) -> &crate::agent::turn_env::HostTurnEnv {
        &self.host_env
    }

    /// Runs this agent as an unattended autonomous tree: sub-agent iteration
    /// caps are lifted to `cap` on every later turn; `None` clears the lift.
    pub fn set_autonomous_iter_cap(&mut self, cap: Option<usize>) {
        self.autonomous_iter_cap = cap;
    }

    pub fn set_event_context(&mut self, session_id: impl Into<String>, channel: impl Into<String>) {
        self.event_session_id = session_id.into();
        self.event_channel = channel.into();
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface();
    }

    /// Route this session's transcripts to `locator` instead of the store its
    /// agent key resolves to. Must be set before the first turn: the session
    /// binds its transcript target once.
    pub fn set_session_history_locator(
        &mut self,
        locator: std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>,
    ) {
        if self.runtime_session.is_some() {
            tracing::debug!("transcript store already bound for this session; keeping it");
            return;
        }
        self.session_history_locator = Some(locator);
    }

    /// Bind the OpenHuman conversation thread for the next and subsequent
    /// turns. Empty input intentionally clears the binding.
    ///
    /// This also binds the session's durable identity, which is what makes a
    /// thread resolve to one transcript instead of a new timestamped stem per
    /// cold boot. A sub-agent is excluded: each spawn is genuinely its own
    /// transcript, and it inherits its parent's thread id only for
    /// correlation.
    pub fn set_thread_id(&mut self, thread_id: Option<impl AsRef<str>>) {
        let thread_id = thread_id.and_then(|thread_id| {
            let thread_id = thread_id.as_ref().trim();
            (!thread_id.is_empty()).then(|| thread_id.to_owned())
        });
        // The runtime session caches its prompt and declaration snapshot from
        // the first turn. Changing identity after that would desynchronize
        // thread-scoped tools from the cached session state.
        if self.runtime_session.is_some() && thread_id != self.thread_id {
            tracing::warn!("cannot change thread id after runtime session initialization");
            return;
        }
        self.thread_id = thread_id;
        self.session = match (&self.thread_id, self.session_parent_prefix.is_some()) {
            (Some(thread_id), false) => Some(tinyagents_session::transcript::SessionRef::scoped(
                thread_id.clone(),
                self.agent_definition_id.clone(),
            )),
            _ => None,
        };
    }

    pub(crate) fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    /// Durable id of the session this host is bound to, or `None` when it has
    /// no conversation identity (a sub-agent, or an unthreaded CLI turn).
    ///
    /// This is TinyAgents' identity, not a host-invented one: it is what
    /// addresses the transcript, and it is stamped into `_meta.session_id`.
    pub fn session_id(&self) -> Option<String> {
        self.session.as_ref().map(|session| session.session_id())
    }

    /// Override the agent definition name used for session transcript
    /// file paths. Callers (e.g. the web channel) use this to scope
    /// transcripts per thread so each conversation thread gets its own
    /// transcript namespace instead of sharing one by agent type.
    ///
    /// Also swaps the agent part of [`Self::session_key`] for the new name,
    /// keeping its timestamp and token.
    pub fn set_agent_definition_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.session_key = crate::agent::session_host::session_key::rename(
            &self.session_key,
            &self.agent_definition_name,
            &name,
        );
        self.agent_definition_name = name;
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface();
    }

    /// Attach a progress event sender for real-time turn updates.
    ///
    /// When set, the turn loop emits [`AgentProgress`] events so
    /// callers (e.g. the web channel) can surface live tool-call and
    /// iteration updates to the UI. Pass `None` to disable.
    pub fn set_on_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    ) {
        // A warm runtime retains the last turn's prelude. Release its sender
        // along with the host sender so the finished turn's bridge can drain
        // and persist the final text without waiting for another turn.
        self.update_runtime_prelude_progress(tx.clone());
        self.on_progress = tx;
    }

    /// Bind this session's acting tools (shell / file / git) to `descriptor`'s
    /// root as their default working directory.
    ///
    /// The post-build counterpart of
    /// [`SessionHostBuilder::workspace_descriptor`](crate::agent::SessionHostBuilder::workspace_descriptor),
    /// for callers that construct the agent through
    /// [`OpenHumanSessionHost::from_config`](crate::agent::OpenHumanSessionHost::from_config) and
    /// therefore never see the builder — notably the per-turn `cwd` of
    /// [`agent_chat`](crate::agent_runs::agent_chat::agent_chat).
    ///
    /// The descriptor is threaded onto the turn's run context, so it also
    /// propagates to sub-agents spawned from this session. `None` restores the
    /// shared `action_dir` cwd.
    ///
    /// This only moves the *default* cwd: what the session may read and write is
    /// still decided by its [`SecurityPolicy`](crate::security::SecurityPolicy),
    /// so a caller that wants tools rooted somewhere new must build the agent
    /// from a config whose `action_dir` already permits it.
    pub fn set_workspace_descriptor(&mut self, descriptor: Option<tinytools::WorkspaceDescriptor>) {
        self.workspace_descriptor = descriptor;
    }

    /// The current workspace binding used by this session's tools.
    pub(crate) fn live_workspace_descriptor(&self) -> Option<tinytools::WorkspaceDescriptor> {
        self.workspace_descriptor.clone()
    }

    /// Attach an active-run queue for mid-turn steering.
    pub fn set_run_queue(
        &mut self,
        rq: Option<
            std::sync::Arc<
                tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>,
            >,
        >,
    ) {
        self.run_queue = rq;
    }
}
