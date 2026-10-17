//! Agent turn origin — the trust/routing label attached to every agent
//! `run_turn` invocation. Read by [`crate::security::approval::ApprovalGate`]
//! and [`tinytools::ToolPolicyEngine`] to make
//! consistent decisions across web, channel, cron, and background entry
//! points without relying on the *absence* of other task-locals as a signal.
//!
//! Every entry point that drives the agent loop ([`crate::web_chat`],
//! [`crate::channels::runtime::dispatch`],
//! [`crate::cron`], CLI) MUST scope a real [`AgentTurnOrigin`]
//! around its `run_turn` invocation. Any path that fails to do so is treated
//! as [`AgentTurnOrigin::Unknown`] by the gate and the call fails closed.

/// Identifies who scheduled the current agent turn so the approval gate can
/// pick the correct policy: surface to the user, persist for an
/// out-of-band approval surface, run trusted-automation through, or fail
/// closed.
///
/// This is a typed task-local label, not a credential — it is set by the
/// entry point that owns the turn and read by [`crate::security::approval`]
/// alongside the existing per-turn chat context.
#[derive(Clone, Debug)]
pub enum AgentTurnOrigin {
    /// Live user chat in the desktop / web UI. The existing
    /// [`crate::security::approval::ApprovalChatContext`] task-local is
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
        /// The sender's display name as the channel gives it (a WhatsApp
        /// push or profile name, a Telegram first and last name), when it
        /// gives one. Never a saved contact name: no channel exposes one.
        sender_name: Option<String>,
        reply_target: String,
        message_id: String,
        /// The channel runtime's conversation-history key for this chat, set only by
        /// the channel processor (the one place that owns `conversation_histories`).
        /// `None` for ExternalChannel-labelled turns that are not a channel
        /// conversation (MCP server callers, triage, voice), so scheduling can tell a
        /// real chat from those.
        history_key: Option<String>,
    },
    /// Internal automation the user explicitly authorized (cron job the
    /// user created, internal background job on local data). `source`
    /// carries enough info for the gate to apply the right per-source
    /// allowlist.
    TrustedAutomation {
        job_id: String,
        source: TrustedAutomationSource,
    },
    /// Command-line / sub-agent / one-off internal invocation.
    Cli,
    /// A person typing into a direct chat surface that is not the web thread —
    /// today the `openhuman.agent_chat` RPC behind the desktop Settings agent
    /// chat panel, and an operator running the same RPC by hand.
    ///
    /// Split out of [`Cli`](Self::Cli) rather than folded into it because the
    /// two answer different questions with the same variant. `Cli` was chosen
    /// for this RPC to tell the **approval gate** "trusted caller, do not fail
    /// closed"; it says nothing about who wrote the text, and its own
    /// documentation covers sub-agent and internal invocations too. Reusing it
    /// for [`is_user_authored`](Self::is_user_authored) would have to answer
    /// "did a person write this" with a trust answer, and would drop a real
    /// person's message on the floor.
    ///
    /// Trust-wise this is exactly `Cli` — the gate treats them identically.
    DirectChat,
    /// Unlabelled — gate fails closed. Every entry point MUST scope a real
    /// origin before invoking the agent.
    Unknown,
}

/// Sub-classification for [`AgentTurnOrigin::TrustedAutomation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustedAutomationSource {
    /// Cron job created and authorized by the user.
    Cron,
    /// Internal background job over locally-stored data (e.g. goals
    /// enrichment). No external content reaches its prompt.
    Background,
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
        /// parks for a real decision, letting a user force human review on a specific flow's outbound
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
            AgentTurnOrigin::DirectChat => "DirectChat".to_string(),
            AgentTurnOrigin::Unknown => "Unknown".to_string(),
        }
    }

    /// Whether the turn's text was written by a **person**.
    ///
    /// `WebChat`, `ExternalChannel`, and `DirectChat` carry what a human sent.
    /// Every other origin carries text the host wrote for an agent to act on: a
    /// `TrustedAutomation` prompt (cron, background job, workflow), a `Cli` invocation — which this module documents as
    /// "command-line / **sub-agent** / one-off internal" — or an unscoped
    /// `Unknown`.
    ///
    /// An allowlist, not a denylist, and for the same reason the permission
    /// gate uses one: a new origin is a turn nobody has classified yet, and
    /// mistaking a host-written prompt for a user message writes it into the
    /// user's memory, where it is indistinguishable from something they said.
    /// A caller that genuinely relays a person's text scopes one of the three
    /// origins above.
    ///
    /// This is a **different question** from the one the approval gate asks,
    /// and the two must not be collapsed onto one variant. The gate asks how
    /// far to trust the caller; this asks who wrote the words. `DirectChat`
    /// exists because `agent_chat` needs the first answer to be "trusted" and
    /// the second to be "a person" — see that variant's note.
    pub fn is_user_authored(&self) -> bool {
        matches!(
            self,
            AgentTurnOrigin::WebChat { .. }
                | AgentTurnOrigin::ExternalChannel { .. }
                | AgentTurnOrigin::DirectChat
        )
    }
}

/// Run `fut` as a turn labelled `origin`.
///
/// The label rides the [`CoreContext`](crate::core::runtime::CoreContext) the
/// future runs under: a derived, immutable copy of the active context (or of
/// the process default, or of a bare one when no core was booted) that carries
/// `origin`. Nothing else holds it, so there is no second copy to drift from
/// the context a spawn carries.
///
/// The inner future is `Box::pin`-ed before it is handed to the context scope
/// so the combined `with_origin(... scope(... run_turn(...)))` future state
/// machine stays heap-allocated. The agent loop downstream of this scope can be
/// deep (tool dispatch, recursive sub-agent invocations, LLM streaming), and
/// stacking scopes plus the agent loop on a 2 MiB worker stack reliably blows
/// the test runtime (same shape as the fix in PR #3151). Boxing here is the
/// single-point remediation that covers every caller (web channel, channel
/// runtime, cron, background jobs, CLI).
pub async fn with_origin<F: std::future::Future>(origin: AgentTurnOrigin, fut: F) -> F::Output {
    let context = crate::core::runtime::CoreContext::scoped_for_turn()
        .or_else(crate::core::runtime::CoreContext::default_context)
        .unwrap_or_else(crate::core::runtime::CoreContext::bare);
    crate::core::runtime::CoreContext::scope_with_turn_origin(context, Some(origin), Box::pin(fut))
        .await
}

/// The origin of the turn running on this task, read from the scoped
/// `CoreContext`. The process default never supplies authority. `None` when no
/// caller scoped one; the gate maps that to [`AgentTurnOrigin::Unknown`]
/// (fail closed).
///
/// The one place the origin is read from task state, for the library and test
/// boundary and for [`capture`]. Production readers take the origin from the
/// context they were handed ([`DispatchCtx::turn_origin`]) or from the tool
/// run carrier.
///
/// [`DispatchCtx::turn_origin`]: crate::core::runtime::DispatchCtx::turn_origin
pub fn current() -> Option<AgentTurnOrigin> {
    crate::core::runtime::DispatchCtx::ambient_turn().turn_origin()
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
/// the scoped `CoreContext` is a `tokio` task-local, so the origin it carries is **lost** the moment
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

/// Carry the origin and the [`CoreContext`](crate::core::runtime::CoreContext)
/// scoped **right now** into a future that will run on another task.
///
/// A `tokio::task_local` does not cross `tokio::spawn`: a detached sub-agent
/// (`spawn_async_subagent`, the orchestration `spawn_agent` task) starts on a
/// fresh task where [`current`] is `None`, so every external-effect tool it
/// calls reaches the approval gate as [`AgentTurnOrigin::Unknown`] and is
/// refused — even though the parent turn that delegated the work was properly
/// labelled. The parent `ParentExecutionContext` and an explicit
/// [`OpenHumanRunContext`](crate::agent::tinyagents::host::OpenHumanRunContext)
/// already travel explicitly to those spawn sites; the origin still has to be
/// carried with them.
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
    // The origin is a field of the scoped context, so carrying the context
    // carries the origin and nothing else: a task with no scoped context stays
    // unlabelled.
    let context = crate::core::runtime::CoreContext::scoped_for_turn();
    async move {
        match context {
            Some(context) => crate::core::runtime::CoreContext::scope(context, fut).await,
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
/// Only the origin (and the ambient `CoreContext`). A delegated agent turn
/// usually also needs the harness fork context, a separate wrapper that still
/// has to be applied around the future passed in here.
pub fn spawn<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    // `propagate` is evaluated here, on the caller's task, which is the whole
    // point of routing through this function.
    tokio::spawn(propagate(fut))
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
