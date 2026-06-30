//! Agent turn origin — the trust/routing label attached to every agent
//! `run_turn` invocation. Read by [`crate::openhuman::approval::ApprovalGate`]
//! and [`crate::openhuman::agent_tool_policy::ToolPolicyEngine`] to make
//! consistent decisions across web, channel, subconscious, and cron entry
//! points without relying on the *absence* of other task-locals as a signal.
//!
//! Every entry point that drives the agent loop ([`crate::openhuman::channels::providers::web`],
//! [`crate::openhuman::channels::runtime::dispatch`], [`crate::openhuman::subconscious`],
//! [`crate::openhuman::cron`], CLI) MUST scope a real [`AgentTurnOrigin`]
//! around its `run_turn` invocation. Any path that fails to do so is treated
//! as [`AgentTurnOrigin::Unknown`] by the gate and the call fails closed.

/// Identifies who scheduled the current agent turn so the approval gate can
/// pick the correct policy: surface to the user, persist for an
/// out-of-band approval surface, run trusted-automation through, or fail
/// closed.
///
/// This is a typed task-local label, not a credential — it is set by the
/// entry point that owns the turn and read by [`crate::openhuman::approval`]
/// alongside the existing per-turn chat context.
#[derive(Clone, Debug)]
pub enum AgentTurnOrigin {
    /// Live user chat in the desktop / web UI. The existing
    /// [`crate::openhuman::approval::ApprovalChatContext`] task-local is
    /// scoped alongside this so the approval gate has a thread / client to
    /// route the prompt back to.
    WebChat {
        thread_id: String,
        client_id: String,
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
}

tokio::task_local! {
    /// Per-turn agent origin. Scoped by entry points (web channel, channel
    /// runtime dispatch, subconscious loop, cron scheduler, CLI) around the
    /// `run_turn` invocation. Read by the approval gate to make
    /// origin-aware decisions.
    pub static AGENT_TURN_ORIGIN: AgentTurnOrigin;
}

/// Scope `origin` for the duration of `fut`. Mirrors the existing
/// [`crate::openhuman::approval::APPROVAL_CHAT_CONTEXT`] scope pattern.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn with_origin_scopes_correctly_and_unscopes_on_exit() {
        // Outside any scope: current() returns None.
        assert!(current().is_none());

        let observed = with_origin(AgentTurnOrigin::Cli, async {
            // Inside the scope: current() returns the scoped origin.
            current()
        })
        .await;
        assert!(matches!(observed, Some(AgentTurnOrigin::Cli)));

        // After the scope exits, current() is None again.
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn current_returns_none_outside_scope() {
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn current_returns_inner_origin_on_nested_scope() {
        let observed = with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: "outer".into(),
                client_id: "c-outer".into(),
            },
            async {
                with_origin(
                    AgentTurnOrigin::TrustedAutomation {
                        job_id: "j-1".into(),
                        source: TrustedAutomationSource::Cron,
                    },
                    async { current() },
                )
                .await
            },
        )
        .await;
        match observed {
            Some(AgentTurnOrigin::TrustedAutomation { job_id, source }) => {
                assert_eq!(job_id, "j-1");
                assert_eq!(source, TrustedAutomationSource::Cron);
            }
            other => panic!("expected inner TrustedAutomation, got {other:?}"),
        }
    }
}
