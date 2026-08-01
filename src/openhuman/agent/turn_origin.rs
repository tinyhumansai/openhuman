//! Agent turn origin — the trust/routing label attached to every agent
//! `run_turn` invocation. Read by [`crate::openhuman::security::approval::ApprovalGate`]
//! and [`crate::openhuman::tools::agent_policy::ToolPolicyEngine`] to make
//! consistent decisions across web, channel, subconscious, and cron entry
//! points without relying on the *absence* of other task-locals as a signal.
//!
//! Every entry point that drives the agent loop ([`crate::openhuman::web_chat`],
//! [`crate::openhuman::channels::runtime::dispatch`],
//! [`crate::openhuman::cron`], CLI) MUST scope a real [`AgentTurnOrigin`]
//! around its `run_turn` invocation. Any path that fails to do so is treated
//! as [`AgentTurnOrigin::Unknown`] by the gate and the call fails closed.

/// Identifies who scheduled the current agent turn so the approval gate can
/// pick the correct policy: surface to the user, persist for an
/// out-of-band approval surface, run trusted-automation through, or fail
/// closed.
///
/// This is a typed task-local label, not a credential — it is set by the
/// entry point that owns the turn and read by [`crate::openhuman::security::approval`]
/// alongside the existing per-turn chat context.
#[derive(Clone, Debug)]
pub enum AgentTurnOrigin {
    /// Live user chat in the desktop / web UI. The existing
    /// [`crate::openhuman::security::approval::ApprovalChatContext`] task-local is
    /// scoped alongside this so the approval gate has a thread / client to
    /// route the prompt back to.
    WebChat {
        thread_id: String,
        client_id: String,
        /// Per-turn request id, when the caller has one. Used by internal
        /// observers to correlate a live progress bridge with the durable
        /// tinyagents journal stream for the same turn.
        request_id: Option<String>,
    },
    /// Inbound message from a non-web channel (Telegram / Discord / Slack /
    /// Yuanbao / etc.). External-effect tools must persist a
    /// `pending_approvals` row for the audit trail; the parked future will
    /// TTL-deny because no caller picks up the chat-routed approval on this
    /// surface yet — which is the correct fail-closed default for remote
    /// inputs.
    ///
    /// `sender` carries the per-user identity (Discord user id, Telegram
    /// from_account, Slack user id, etc.) when available so per-user
    /// isolation invariants survive into the gate's audit trail. Legacy
    /// publishers that don't surface the sender pass `None`; the gate still
    /// fails closed because the channel input is remote-untrusted regardless
    /// of which sender produced it. Distinct senders in the same shared
    /// channel produce distinct origins so a co-channel attacker cannot
    /// resume a victim's parked approval flow.
    ExternalChannel {
        channel: String,
        sender: Option<String>,
        reply_target: String,
        message_id: String,
    },
    /// Internal automation the user explicitly authorized (cron job the
    /// user created, subconscious tick on internal-only memory). `source`
    /// carries enough info for the gate to apply the right per-source
    /// allowlist.
    TrustedAutomation {
        job_id: String,
        source: TrustedAutomationSource,
    },
    /// Command-line / sub-agent / one-off internal invocation.
    Cli,
    /// Unlabelled — gate fails closed. Every entry point MUST scope a real
    /// origin before invoking the agent.
    Unknown,
}

/// Sub-classification for [`AgentTurnOrigin::TrustedAutomation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustedAutomationSource {
    /// Cron job created and authorized by the user.
    Cron,
    /// Subconscious tick whose memory context is internal-only.
    Subconscious,
    /// Subconscious tick whose memory context includes chunks ingested
    /// from an external sync source (Gmail / Slack / Notion / etc.).
    /// Treated as untrusted: external-effect tool surface blocked.
    SubconsciousTainted,
    /// Autonomous continuation of a thread goal: the heartbeat injected a turn
    /// to keep working an idle `active` goal the user explicitly created.
    GoalContinuation,
    /// A saved, enabled `flows::Flow` (tinyflows workflow) executing via
    /// `flows::ops::flows_run` / `flows_resume` (issue B2, see
    /// `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §3). The
    /// flow's `tool_call`/`http_request` nodes were pre-declared (their
    /// `slug`/`url` are static graph config, never `=`-expression evaluated
    /// in tinyflows 0.2 — see `my_docs/ohxtf/commons/12-node-catalog-0.2.md`)
    /// and validated when the flow was saved, so the *action* carries a trust
    /// root the same way a user-authored cron job's prompt does. The runtime
    /// trigger payload (webhook body, Composio event, …) stays untrusted —
    /// nothing in it can introduce a *new* action, only feed the pre-declared
    /// one's arguments.
    Workflow {
        /// Mirrors `Flow::require_approval`: when `true` the gate does NOT
        /// auto-allow this trust root — every external_effect call still
        /// parks for a real decision (same shape as `GoalContinuation`),
        /// letting a user force human review on a specific flow's outbound
        /// actions regardless of the trust root above.
        require_approval: bool,
    },
}

impl AgentTurnOrigin {
    /// A PII-free classification label safe for `info`-level logs and audit
    /// trails — the variant name (and, for `TrustedAutomation`, its `source`
    /// sub-kind), never an identifying field. Use this instead of `{:?}` /
    /// `?origin` anywhere the log line isn't gated to `debug`/`trace`:
    /// `WebChat.thread_id`/`client_id`, `ExternalChannel.sender`/
    /// `reply_target`/`message_id`, and `TrustedAutomation.job_id` can carry
    /// user- or channel-identifying data that must not land at `info`.
    pub fn class(&self) -> String {
        match self {
            AgentTurnOrigin::WebChat { .. } => "WebChat".to_string(),
            AgentTurnOrigin::ExternalChannel { channel, .. } => {
                format!("ExternalChannel({channel})")
            }
            AgentTurnOrigin::TrustedAutomation { source, .. } => {
                format!("TrustedAutomation({source:?})")
            }
            AgentTurnOrigin::Cli => "Cli".to_string(),
            AgentTurnOrigin::Unknown => "Unknown".to_string(),
        }
    }

    /// Whether the turn's text was written by a **person**.
    ///
    /// `WebChat` and `ExternalChannel` carry what a human sent. Every other
    /// origin carries text the host wrote for an agent to act on: a
    /// `TrustedAutomation` prompt (cron, subconscious, goal continuation,
    /// workflow), a `Cli` invocation — which this module documents as
    /// "command-line / **sub-agent** / one-off internal" — or an unscoped
    /// `Unknown`.
    ///
    /// An allowlist, not a denylist, and for the same reason the permission
    /// gate uses one: a new origin is a turn nobody has classified yet, and
    /// mistaking a host-written prompt for a user message writes it into the
    /// user's memory, where it is indistinguishable from something they said.
    /// A caller that genuinely relays a person's text scopes one of the two
    /// origins above.
    pub fn is_user_authored(&self) -> bool {
        matches!(
            self,
            AgentTurnOrigin::WebChat { .. } | AgentTurnOrigin::ExternalChannel { .. }
        )
    }
}

/// Whether the current turn's text was written by a person — `false` outside
/// any origin scope, matching [`AgentTurnOrigin::is_user_authored`]'s allowlist.
pub fn current_is_user_authored() -> bool {
    current().is_some_and(|origin| origin.is_user_authored())
}

tokio::task_local! {
    /// Per-turn agent origin. Scoped by entry points (web channel, channel
    /// runtime dispatch, subconscious loop, cron scheduler, CLI) around the
    /// `run_turn` invocation. Read by the approval gate to make
    /// origin-aware decisions.
    pub static AGENT_TURN_ORIGIN: AgentTurnOrigin;
}

/// Scope `origin` for the duration of `fut`. Mirrors the existing
/// [`crate::openhuman::security::approval::APPROVAL_CHAT_CONTEXT`] scope pattern.
///
/// The inner future is `Box::pin`-ed before being handed to the task-local
/// scope so the combined `with_origin(... scope(... run_turn(...)))` future
/// state machine stays heap-allocated. The agent loop downstream of this
/// scope can be deep (tool dispatch, recursive sub-agent invocations, LLM
/// streaming), and stacking two task-local scopes plus the agent loop on a
/// 2 MiB worker stack reliably blows the test runtime — same shape as the
/// fix in PR #3151. Box-pinning here is the single-point remediation that
/// covers every caller (web channel, channel runtime, subconscious, cron,
/// CLI).
pub async fn with_origin<F: std::future::Future>(origin: AgentTurnOrigin, fut: F) -> F::Output {
    AGENT_TURN_ORIGIN.scope(origin, Box::pin(fut)).await
}

/// Try to read the current origin. Returns `None` when no caller scoped one
/// (legacy callers that haven't been migrated yet — the gate maps this to
/// [`AgentTurnOrigin::Unknown`] / fail-closed).
pub fn current() -> Option<AgentTurnOrigin> {
    AGENT_TURN_ORIGIN.try_with(|o| o.clone()).ok()
}

/// Capture the ambient origin so it can be carried across a `tokio::spawn`
/// boundary by [`with_inherited_origin`].
///
/// This is exactly [`current()`] — it exists as a named pair with
/// `with_inherited_origin` so the capture/re-scope idiom is greppable at every
/// delegation site, and so the capture is obviously required to happen on the
/// *parent* task (task-locals do not cross `tokio::spawn`; calling this inside
/// the spawned future always yields `None`).
pub fn capture() -> Option<AgentTurnOrigin> {
    current()
}

/// Re-scope a [`capture()`]d origin around `fut` on a freshly-spawned task.
///
/// # Why this is inherit-only
///
/// `AGENT_TURN_ORIGIN` is a `tokio` task-local, so it is **lost** the moment
/// work moves onto a new task via `tokio::spawn`. An async sub-agent, team
/// member, or workflow phase therefore runs unlabelled, the approval gate reads
/// [`AgentTurnOrigin::Unknown`], and every `external_effect` tool (shell/exec)
/// is refused. Re-establishing the parent's label is the fix.
///
/// It re-establishes the parent's label and **nothing else**:
///
/// * `Some(origin)` — scope that exact origin, unchanged. A worker descending
///   from an [`AgentTurnOrigin::ExternalChannel`] turn stays `ExternalChannel`
///   (remote, untrusted); it is never promoted to `Cli` or any other origin
///   just because it now runs on a background task. Delegation must not be a
///   privilege-escalation primitive.
/// * `None` — run `fut` with **no** scope at all. The spawned task stays
///   unlabelled and the gate keeps failing closed exactly as it does today.
///   Never substitute a default origin here: fabricating a label for an
///   unlabelled parent would hand every unlabelled call site in the process a
///   trust root it never earned.
///
/// Capture on the parent task *before* the `tokio::spawn`, move the
/// `Option<AgentTurnOrigin>` into the spawned future, and wrap the future's
/// body:
///
/// ```ignore
/// let inherited = turn_origin::capture();
/// tokio::spawn(async move {
///     turn_origin::with_inherited_origin(inherited, async move { /* agent work */ }).await
/// });
/// ```
pub async fn with_inherited_origin<F: std::future::Future>(
    captured: Option<AgentTurnOrigin>,
    fut: F,
) -> F::Output {
    match captured {
        // Box-pinned by `with_origin` for the same stack-depth reason
        // documented there — the agent loop downstream can be very deep.
        Some(origin) => with_origin(origin, fut).await,
        // Deliberately unlabelled: fail-closed is the correct default.
        None => fut.await,
    }
}

/// Carry the origin scoped **right now** into a future that will run on
/// another task.
///
/// A `tokio::task_local` does not cross `tokio::spawn`: a detached sub-agent
/// (`spawn_async_subagent`, the orchestration `spawn_agent` task) starts on a
/// fresh task where [`current`] is `None`, so every external-effect tool it
/// calls reaches the approval gate as [`AgentTurnOrigin::Unknown`] and is
/// refused — even though the parent turn that delegated the work was properly
/// labelled. That is the same failure mode
/// [`fork_context::with_parent_context`](crate::openhuman::agent::harness::fork_context)
/// and [`thread_context::with_thread_id`](crate::openhuman::agent::tinyagents::thread_context)
/// already re-install explicitly at those spawn sites; the origin is the third
/// thing that has to travel with them.
///
/// **Call this on the parent task**, i.e. build the future *before* handing it
/// to `tokio::spawn` — the origin is read when this function is called, not
/// when the returned future is first polled:
///
/// ```ignore
/// tokio::spawn(turn_origin::propagate(async move { run_subagent(..).await }));
/// ```
///
/// Fail-closed is preserved: with no ambient origin nothing is scoped, so the
/// child still lands on `Unknown` rather than inheriting a label nobody set.
/// This only ever *carries* a decision the parent entry point already made — it
/// cannot manufacture trust that did not exist on the spawning task.
pub fn propagate<F: std::future::Future>(fut: F) -> impl std::future::Future<Output = F::Output> {
    let captured = current();
    async move {
        match captured {
            Some(origin) => with_origin(origin, fut).await,
            None => fut.await,
        }
    }
}

/// `tokio::spawn`, with the current turn origin carried onto the new task.
///
/// # Why this exists when [`propagate`] already does the carrying
///
/// [`propagate`] and [`capture`] read the origin **when they are called**, which
/// has to be on the spawning task — a task-local is already gone by the time the
/// spawned future is first polled. Both of these compile, neither warns, and
/// only the first is right:
///
/// ```ignore
/// tokio::spawn(turn_origin::propagate(work));              // correct
/// tokio::spawn(async move { turn_origin::propagate(work).await });  // silently Unknown
/// ```
///
/// The second captures inside the new task, where [`current`] is already `None`,
/// so it scopes nothing and every external-effect tool the child calls is
/// refused by the approval gate. The existing call sites get this right only
/// because each one carries a hand-written comment saying to capture *here, on
/// the spawning task* — correctness resting on reviewer attention at every
/// future site.
///
/// This helper removes the ordering from the caller's hands: the capture happens
/// inside, before the spawn, and there is no argument order that can get it
/// wrong.
///
/// # Fail-closed is preserved
///
/// With no ambient origin nothing is scoped, so the child lands on
/// [`AgentTurnOrigin::Unknown`] exactly as a bare `tokio::spawn` would. This
/// only ever *carries* a decision some entry point already made; it cannot
/// manufacture a trust root. See [`propagate`], which does the actual work.
///
/// # What it does not carry
///
/// Only the origin. A delegated agent turn usually also needs
/// [`turn_workspace::propagate`](super::turn_workspace) and the harness fork
/// context; those are separate wrappers and still have to be applied around the
/// future passed in here.
///
/// ```ignore
/// let join = turn_origin::spawn(turn_workspace::propagate(async move { .. }));
/// ```
pub fn spawn<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    // `propagate` is evaluated here, on the caller's task, which is the whole
    // point of routing through this function.
    tokio::spawn(propagate(fut))
}

/// `tokio::spawn` for work that must deliberately **not** carry the caller's
/// origin, naming why.
///
/// Dropping the origin is sometimes right — a detached background job that is
/// not a continuation of the caller's turn should not inherit that turn's
/// authority. The problem is that a bare `tokio::spawn` looks identical whether
/// the author decided that or simply did not think about it, so a reviewer
/// cannot tell a deliberate choice from a regression.
///
/// This is a plain `tokio::spawn` — the behaviour is the same — but the name and
/// the `reason` make the choice explicit at the call site and greppable across
/// the tree. The reason is emitted at `trace` so a live process can be asked
/// which spawns dropped their label.
///
/// Prefer [`spawn`] unless the work genuinely is not a continuation of the
/// caller's turn.
pub fn spawn_unlabelled<F>(reason: &'static str, fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tracing::trace!(
        reason,
        parent_origin = ?current().as_ref().map(AgentTurnOrigin::class),
        "[turn_origin] spawning without the caller's origin"
    );
    tokio::spawn(fut)
}

/// Read the ambient web-chat `request_id` for the current turn, when one was
/// scoped by an [`AgentTurnOrigin::WebChat`] entry point. `None` for every
/// other origin (channel / cron / CLI / sub-agent) and outside any scope —
/// those turns are not request-scoped, so their transcript lines carry no
/// turn-boundary marker.
pub fn current_request_id() -> Option<String> {
    match current() {
        Some(AgentTurnOrigin::WebChat { request_id, .. }) => request_id,
        _ => None,
    }
}

#[cfg(test)]
#[path = "turn_origin_tests.rs"]
mod tests;
