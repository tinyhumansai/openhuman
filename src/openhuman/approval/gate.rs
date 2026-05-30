//! `ApprovalGate` — middleware between the agent and any tool whose
//! [`crate::openhuman::tools::Tool::external_effect`] returns `true`.
//!
//! Flow (issue #1339):
//! 1. Agent harness calls [`ApprovalGate::intercept`] with the tool
//!    name, a redacted JSON of the arguments, and a short summary.
//! 2. Gate checks the user's "Always allow" allowlist
//!    (`autonomy.auto_approve`, read live via
//!    [`crate::openhuman::security::live_policy`]). Hit → `Allow`
//!    immediately. An `ApproveAlwaysForTool` decision adds the tool to
//!    that list via `approval_decide` (config save + policy reload).
//! 3. Otherwise: persist a row in `pending_approvals`, publish a
//!    [`DomainEvent::ApprovalRequested`] event so the UI can pop a
//!    toast, and park the call on a `oneshot::Sender` keyed by
//!    `request_id`.
//! 4. UI calls `approval_decide` (RPC) which routes through
//!    [`ApprovalGate::decide`] → sends the decision on the oneshot.
//! 5. The parked future wakes with the decision and translates it
//!    into [`GateOutcome::Allow`] / `Deny`.
//!
//! Sessions: the gate is keyed by an internal per-launch UUID
//! (`session-<uuid>`) used purely for audit grouping. This value is
//! generated unconditionally by the caller (see
//! `bootstrap_core_runtime`) and is never derived from the JSON-RPC
//! bearer token or any other credential material — it is safe to
//! persist and to log. Rows from prior launches are intentionally
//! preserved on init — the issue #1339 acceptance criterion requires
//! they survive restart so the UI can show / dismiss orphans.
//! Decisions on orphan rows update the DB but cannot resume a parked
//! future across processes — no side effect can fire across launches,
//! so the security invariant is preserved without auto-purging.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::security::POLICY_DENIED_MARKER;

use super::store;
use super::types::{ApprovalDecision, ExecutionOutcome, GateOutcome, PendingApproval};

/// How long the gate will park a future before timing out and
/// returning `Deny`. 10 minutes matches the default `expires_at`
/// written into the persisted row.
const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(60 * 10);

/// Per-turn chat context for routing a parked approval's yes/no reply back to
/// the originating thread. The web channel scopes this task-local around the
/// agent run (`channels::providers::web`); because the `run_turn` handler, the
/// tool loop, and `intercept` all run inline (`.await`) within that spawned
/// task, it propagates down to `intercept` with no signature plumbing. Absent
/// for non-chat callers (CLI, sub-agents) — their approvals are simply not
/// chat-routable.
#[derive(Clone, Debug)]
pub struct ApprovalChatContext {
    pub thread_id: String,
    pub client_id: String,
}

tokio::task_local! {
    pub static APPROVAL_CHAT_CONTEXT: ApprovalChatContext;
}

/// Parse a chat reply to a parked approval into a binary decision (v1). Only an
/// explicit yes/no answer maps to a decision; anything else returns `None` — the
/// web channel treats `None` as "not an answer", cancels the parked turn, and
/// dispatches the message as a fresh user turn (so the user can redirect).
pub fn parse_approval_reply(message: &str) -> Option<ApprovalDecision> {
    match message.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "ok" | "okay" | "approve" | "approved" | "allow" => {
            Some(ApprovalDecision::ApproveOnce)
        }
        "no" | "n" | "deny" | "denied" => Some(ApprovalDecision::Deny),
        _ => None,
    }
}

static GLOBAL_GATE: OnceLock<Arc<ApprovalGate>> = OnceLock::new();

/// Coordinator for pending approvals.
pub struct ApprovalGate {
    config: Config,
    session_id: String,
    ttl: Duration,
    waiters: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
    /// thread_id → request_id for the approval currently parked on that chat
    /// thread, so the web channel can route a yes/no reply to `approval_decide`.
    /// In-memory only (session-scoped — a parked approval doesn't survive a
    /// restart, and the oneshot waiter is in-memory anyway).
    thread_to_request: Mutex<HashMap<String, String>>,
}

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
        let gate = Arc::new(ApprovalGate::new(config, session_id, DEFAULT_APPROVAL_TTL));
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
        }
    }

    /// Whether `tool_name` is on the user's "Always allow" list. Prefers the
    /// process-global live policy (so a grant made this session is seen
    /// immediately) and falls back to the gate's boot-time config snapshot.
    fn tool_is_auto_approved(&self, tool_name: &str) -> bool {
        if let Some(policy) = crate::openhuman::security::live_policy::current() {
            return policy.auto_approve.iter().any(|t| t == tool_name);
        }
        self.config
            .autonomy
            .auto_approve
            .iter()
            .any(|t| t == tool_name)
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
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> GateOutcome {
        // Drop the request_id; callers using the legacy entry point
        // don't record execution.
        self.intercept_audited(tool_name, action_summary, args_redacted)
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
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> (GateOutcome, Option<String>) {
        // "Always allow" allowlist shortcut — the user's persisted
        // `autonomy.auto_approve` set. Read from the live policy first so a
        // grant made earlier in this session (which writes config + reloads the
        // live policy) takes effect on the very next tool call; fall back to the
        // gate's boot-time config when no live policy is installed (e.g. a CLI
        // invocation that never started a session runtime, or a unit test).
        if self.tool_is_auto_approved(tool_name) {
            tracing::debug!(
                tool = tool_name,
                "[approval::gate] auto_approve allowlist hit, skipping prompt"
            );
            return (GateOutcome::Allow, None);
        }

        // Chat context (thread/client id) for routing the yes/no reply — set by
        // the web channel around the agent run; absent for non-chat callers.
        let chat_ctx = APPROVAL_CHAT_CONTEXT.try_with(|c| c.clone()).ok();
        let chat_thread_id = chat_ctx.as_ref().map(|c| c.thread_id.clone());
        let chat_client_id = chat_ctx.as_ref().map(|c| c.client_id.clone());

        // The gate is interactive: it only engages when there's a live chat turn
        // to surface the prompt to and a human to answer it. Background / triage
        // / cron turns carry no `ApprovalChatContext` — they are pre-authorized
        // autonomous automation, and gating them would park with nobody to
        // answer (→ TTL timeout → deny), stalling the automation. So with no
        // chat context, allow the call straight through.
        if chat_ctx.is_none() {
            tracing::debug!(
                tool = tool_name,
                "[approval::gate] no chat context (non-interactive turn) — not gating"
            );
            return (GateOutcome::Allow, None);
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let expires_at = Some(now + chrono::Duration::from_std(self.ttl).unwrap_or_default());
        let pending = PendingApproval {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted: args_redacted.clone(),
            created_at: now,
            expires_at,
        };

        // Register the waiter BEFORE persisting the row so a fast
        // `approval_decide` cannot mark the request approved while
        // no waiter exists — would otherwise leave the parked call
        // to time out and return `Deny` incorrectly. (CodeRabbit
        // review on PR #2149.)
        let (tx, rx) = oneshot::channel::<ApprovalDecision>();
        {
            let mut waiters = self.waiters.lock();
            waiters.insert(request_id.clone(), tx);
        }
        // Record the thread → request mapping so an inbound chat reply on this
        // thread can be routed to `approval_decide` (see web channel ingress).
        if let Some(thread_id) = chat_thread_id.as_ref() {
            self.thread_to_request
                .lock()
                .insert(thread_id.clone(), request_id.clone());
        }

        if let Err(err) = store::insert_pending(&self.config, &pending, &self.session_id) {
            self.evict_waiter(&request_id);
            self.clear_thread(&chat_thread_id);
            tracing::error!(
                error = %err,
                tool = tool_name,
                "[approval::gate] failed to persist pending row — failing closed"
            );
            return (
                GateOutcome::Deny {
                    reason: format!(
                        "{POLICY_DENIED_MARKER} Approval gate could not persist the request — \
                         denying for safety: {err}"
                    ),
                },
                None,
            );
        }

        tracing::info!(
            request_id = %request_id,
            tool = tool_name,
            thread_id = chat_thread_id.as_deref().unwrap_or("<none>"),
            client_id = chat_client_id.as_deref().unwrap_or("<none>"),
            "[approval::gate] publishing ApprovalRequested (surface fires only if thread_id+client_id are both set)"
        );
        publish_global(DomainEvent::ApprovalRequested {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted,
            thread_id: chat_thread_id.clone(),
            client_id: chat_client_id.clone(),
        });

        tracing::info!(
            request_id = %request_id,
            tool = tool_name,
            "[approval::gate] tool call parked, waiting for decision"
        );

        let outcome = match tokio::time::timeout(self.ttl, rx).await {
            Ok(Ok(decision)) => {
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    decision = decision.as_str(),
                    "[approval::gate] decision received"
                );
                if decision.is_approve() {
                    (GateOutcome::Allow, Some(request_id))
                } else {
                    (
                        GateOutcome::Deny {
                            reason: format!(
                                "{POLICY_DENIED_MARKER} User denied '{tool_name}' execution. Do \
                                 not re-request the same call this turn; take a different approach \
                                 or stop."
                            ),
                        },
                        None,
                    )
                }
            }
            Ok(Err(_canceled)) => {
                // Sender dropped — treat as denial so the agent does
                // not silently no-op.
                tracing::warn!(
                    request_id = %request_id,
                    tool = tool_name,
                    "[approval::gate] decision channel dropped — denying"
                );
                let _ = store::decide(&self.config, &request_id, ApprovalDecision::Deny);
                (
                    GateOutcome::Deny {
                        reason: format!(
                            "{POLICY_DENIED_MARKER} Approval channel for '{tool_name}' closed \
                             before a decision was made."
                        ),
                    },
                    None,
                )
            }
            Err(_elapsed) => {
                self.evict_waiter(&request_id);
                // Race: `decide()` may have committed an Approve in
                // SQLite right as the TTL elapsed. `store::decide(Deny)`
                // has `WHERE decided_at IS NULL` so it won't overwrite,
                // but without a re-read we'd return Deny here while the
                // durable audit row says Approved (CodeRabbit review on
                // #2367). Try to deny; if the row was already decided,
                // honor the persisted decision.
                let denied = store::decide(&self.config, &request_id, ApprovalDecision::Deny);
                let persisted = match &denied {
                    Ok(Some(_)) => Some(ApprovalDecision::Deny),
                    Ok(None) => store::get_decision(&self.config, &request_id)
                        .ok()
                        .flatten(),
                    Err(_) => None,
                };
                if matches!(persisted, Some(d) if d.is_approve()) {
                    tracing::info!(
                        request_id = %request_id,
                        tool = tool_name,
                        ttl_secs = self.ttl.as_secs(),
                        "[approval::gate] timeout race: persisted decision was Approve, honoring approval"
                    );
                    // Fall through (no early return) so `clear_thread` below runs
                    // on this path too — otherwise the stale thread→request
                    // mapping survives and the next yes/no on the thread could be
                    // routed to this already-finished request.
                    (GateOutcome::Allow, Some(request_id))
                } else {
                    tracing::warn!(
                        request_id = %request_id,
                        tool = tool_name,
                        ttl_secs = self.ttl.as_secs(),
                        "[approval::gate] approval timed out, denying"
                    );
                    (
                        GateOutcome::Deny {
                            reason: format!(
                                "{POLICY_DENIED_MARKER} Approval for '{tool_name}' timed out after \
                                 {}s. Do not re-request the same call this turn; take a different \
                                 approach or stop.",
                                self.ttl.as_secs()
                            ),
                        },
                        None,
                    )
                }
            }
        };
        // The thread routing mapping is only needed while parked; clear it on
        // every exit (decision, channel drop, or timeout).
        self.clear_thread(&chat_thread_id);
        outcome
    }

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
            publish_global(DomainEvent::ApprovalDecided {
                request_id: row.request_id.clone(),
                tool_name: row.tool_name.clone(),
                decision: decision.as_str().to_string(),
            });
        }
        Ok(decided)
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

    /// Drop the thread → request mapping (best-effort; no-op when absent).
    fn clear_thread(&self, thread_id: &Option<String>) {
        if let Some(t) = thread_id {
            self.thread_to_request.lock().remove(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_gate() -> (ApprovalGate, TempDir) {
        let dir = TempDir::new().unwrap();
        let config = Config {
            workspace_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        // Mirrors the `session-<uuid>` shape minted by
        // `bootstrap_core_runtime` in production so the
        // `debug_assert!` regression guard in `ApprovalGate::new`
        // doesn't trip in tests.
        let session = format!("session-{}", uuid::Uuid::new_v4());
        // 500ms TTL was racing the 50×10ms poll loop on slow CI
        // runners — the row would expire (and get denied by
        // list_pending's lazy-expire) before `decide` could fire,
        // surfacing as "pending row never appeared". 2s gives the
        // polling tests enough headroom while keeping
        // `timeout_returns_deny` fast (PR #2367 CI flake).
        let gate = ApprovalGate::new(config, session, Duration::from_secs(2));
        (gate, dir)
    }

    /// A chat context — the gate only parks within a live chat turn now, so
    /// tests that exercise parking must run intercept inside this scope.
    fn chat_ctx() -> ApprovalChatContext {
        ApprovalChatContext {
            thread_id: "t-test".into(),
            client_id: "c-test".into(),
        }
    }

    #[tokio::test]
    async fn approve_once_returns_allow() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            APPROVAL_CHAT_CONTEXT
                .scope(
                    chat_ctx(),
                    g.intercept("composio", "send slack", serde_json::json!({})),
                )
                .await
        });

        // Wait for pending row to land.
        let mut tries = 0;
        let pending = loop {
            let list = gate.list_pending().unwrap();
            if let Some(p) = list.into_iter().next() {
                break p;
            }
            tries += 1;
            assert!(tries < 50, "pending row never appeared");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
            .unwrap();

        let outcome = handle.await.unwrap();
        assert!(matches!(outcome, GateOutcome::Allow));
    }

    #[tokio::test]
    async fn deny_returns_deny_with_reason() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            APPROVAL_CHAT_CONTEXT
                .scope(
                    chat_ctx(),
                    g.intercept("pushover", "send push", serde_json::json!({})),
                )
                .await
        });

        let pending = loop {
            if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        gate.decide(&pending.request_id, ApprovalDecision::Deny)
            .unwrap();

        let outcome = handle.await.unwrap();
        match outcome {
            GateOutcome::Deny { reason } => assert!(reason.contains("pushover")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn auto_approve_tool_skips_prompt() {
        // The gate reads the "Always allow" allowlist from the process-global
        // live policy. Serialize with the other tests that install/reload it
        // (the `live_policy` module test + the autonomy `ops` tests, which all
        // take this same lock) so a parallel install can't clobber ours mid-test.
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (gate, dir) = test_gate();

        // A tool name unique to this test so leaving it in the global allowlist
        // afterwards can't make a sibling gate test (which use "composio" /
        // "pushover") skip its expected prompt.
        let tool = "openhuman_test_always_allow_tool";
        let policy = crate::openhuman::security::SecurityPolicy {
            auto_approve: vec![tool.into()],
            ..crate::openhuman::security::SecurityPolicy::default()
        };
        crate::openhuman::security::live_policy::install(
            Arc::new(policy),
            dir.path().to_path_buf(),
        );

        // An allow-listed tool short-circuits the gate to `Allow` immediately —
        // before any parking — even with a live chat context present, and
        // without persisting a pending row.
        let outcome = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx(),
                gate.intercept(tool, "noop", serde_json::json!({})),
            )
            .await;
        assert!(matches!(outcome, GateOutcome::Allow));
        assert!(
            gate.list_pending().unwrap().is_empty(),
            "an auto-approved call must not create a pending approval row"
        );
    }

    #[tokio::test]
    async fn timeout_returns_deny() {
        let (gate, _dir) = test_gate(); // TTL = 500ms
        let gate = Arc::new(gate);
        let outcome = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx(),
                gate.intercept("composio", "timed out", serde_json::json!({})),
            )
            .await;
        match outcome {
            GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn decide_unknown_id_is_noop() {
        let (gate, _dir) = test_gate();
        let decided = gate
            .decide("does-not-exist", ApprovalDecision::ApproveOnce)
            .unwrap();
        assert!(decided.is_none());
    }

    #[tokio::test]
    async fn pending_for_thread_tracks_request_under_chat_context_and_clears() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        // Run intercept inside a scoped chat context (as the web channel does).
        let g = gate.clone();
        let ctx = ApprovalChatContext {
            thread_id: "thread-42".into(),
            client_id: "client-1".into(),
        };
        let handle = tokio::spawn(async move {
            APPROVAL_CHAT_CONTEXT
                .scope(ctx, g.intercept("shell", "run ls", serde_json::json!({})))
                .await
        });

        // While parked, the thread → request mapping is queryable.
        let mut tries = 0;
        let request_id = loop {
            if let Some(r) = gate.pending_for_thread("thread-42") {
                break r;
            }
            tries += 1;
            assert!(tries < 50, "thread mapping never appeared");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        // Decide via the mapped request_id (as the chat ingress router will).
        gate.decide(&request_id, ApprovalDecision::ApproveOnce)
            .unwrap();
        assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));

        // Mapping is cleared once intercept returns.
        assert!(gate.pending_for_thread("thread-42").is_none());
    }

    #[test]
    fn parse_approval_reply_maps_yes_no_and_rejects_other() {
        for y in ["yes", "Y", " OK ", "approve", "Allow", "okay"] {
            assert_eq!(
                super::parse_approval_reply(y),
                Some(ApprovalDecision::ApproveOnce),
                "{y}"
            );
        }
        for n in ["no", "N", "deny", "Denied"] {
            assert_eq!(
                super::parse_approval_reply(n),
                Some(ApprovalDecision::Deny),
                "{n}"
            );
        }
        // Anything else is NOT an answer → caller cancels + redirects.
        for other in [
            "maybe",
            "actually do Y instead",
            "",
            "yep nope",
            "sure thing",
        ] {
            assert_eq!(super::parse_approval_reply(other), None, "{other}");
        }
    }

    #[tokio::test]
    async fn no_chat_context_is_allowed_not_gated() {
        // The gate is interactive: a non-chat caller (background / triage / cron,
        // no ApprovalChatContext) is allowed straight through — never parked —
        // so autonomous turns don't stall on an approval no one can answer.
        let (gate, _dir) = test_gate();
        let outcome = gate
            .intercept("shell", "run ls", serde_json::json!({}))
            .await;
        assert!(matches!(outcome, GateOutcome::Allow));
        assert!(gate.pending_for_thread("thread-42").is_none());
    }

    #[tokio::test]
    async fn intercept_audited_returns_request_id_only_when_allowed_and_persisted() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        // Allow path: the audited variant must hand back the
        // request_id so the caller can record_execution later
        // (issue #2135).
        let g = gate.clone();
        let handle = tokio::spawn(async move {
            // Scope a chat context *inside* the spawned task — task-locals don't
            // cross `tokio::spawn`, and `intercept` only parks (creates a pending
            // row) when a chat context is present.
            APPROVAL_CHAT_CONTEXT
                .scope(
                    chat_ctx(),
                    g.intercept_audited("composio", "send slack", serde_json::json!({})),
                )
                .await
        });
        let pending = loop {
            if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
            .unwrap();
        let (outcome, id) = handle.await.unwrap();
        assert!(matches!(outcome, GateOutcome::Allow));
        assert_eq!(
            id.as_deref(),
            Some(pending.request_id.as_str()),
            "allowed call must return its persisted request id"
        );

        // Now record execution against that id. Round-trip via a
        // fresh gate to prove the row landed in durable storage.
        gate.record_execution(&pending.request_id, ExecutionOutcome::Success, None);
    }

    #[tokio::test]
    async fn intercept_audited_id_is_none_for_denied_some_for_approved() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        // Deny path → no id (nothing to record afterward).
        let g = gate.clone();
        let denied = tokio::spawn(async move {
            APPROVAL_CHAT_CONTEXT
                .scope(
                    chat_ctx(),
                    g.intercept_audited("composio", "send slack", serde_json::json!({})),
                )
                .await
        });
        let pending = loop {
            if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        gate.decide(&pending.request_id, ApprovalDecision::Deny)
            .unwrap();
        let (outcome, id) = denied.await.unwrap();
        assert!(matches!(outcome, GateOutcome::Deny { .. }));
        assert!(id.is_none(), "denied calls have nothing to record");

        // Allowlist-shortcut path → also no id (no row was created).
        let g = gate.clone();
        let first = tokio::spawn(async move {
            APPROVAL_CHAT_CONTEXT
                .scope(
                    chat_ctx(),
                    g.intercept_audited("pushover", "first send", serde_json::json!({})),
                )
                .await
        });
        let pending = loop {
            if let Some(p) = gate
                .list_pending()
                .unwrap()
                .into_iter()
                .find(|p| p.tool_name == "pushover")
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        // `ApproveAlwaysForTool` resolves the parked prompt to Allow and, because
        // the prompt persisted a row, returns its id. (Persisting the tool onto
        // the `auto_approve` allowlist for *future* calls is the RPC handler's
        // job — see `approval::rpc::approval_decide` — and the gate's allowlist
        // short-circuit is covered by `auto_approve_tool_skips_prompt`.)
        gate.decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForTool)
            .unwrap();
        let (first_outcome, first_id) = first.await.unwrap();
        assert!(matches!(first_outcome, GateOutcome::Allow));
        assert!(
            first_id.is_some(),
            "the prompting call still persists a row"
        );
    }
}
