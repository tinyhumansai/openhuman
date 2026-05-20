//! `ApprovalGate` — middleware between the agent and any tool whose
//! [`crate::openhuman::tools::Tool::external_effect`] returns `true`.
//!
//! Flow (issue #1339):
//! 1. Agent harness calls [`ApprovalGate::intercept`] with the tool
//!    name, a redacted JSON of the arguments, and a short summary.
//! 2. Gate checks the session-scoped allowlist (built from prior
//!    `ApproveAlwaysForTool` decisions). Hit → `Allow` immediately.
//! 3. Otherwise: persist a row in `pending_approvals`, publish a
//!    [`DomainEvent::ApprovalRequested`] event so the UI can pop a
//!    toast, and park the call on a `oneshot::Sender` keyed by
//!    `request_id`.
//! 4. UI calls `approval_decide` (RPC) which routes through
//!    [`ApprovalGate::decide`] → sends the decision on the oneshot.
//! 5. The parked future wakes with the decision and translates it
//!    into [`GateOutcome::Allow`] / `Deny`.
//!
//! Sessions: the gate is keyed by a per-launch `session_id` (the
//! per-launch hex bearer the core hands out) for audit grouping.
//! Rows from prior launches are intentionally preserved on init —
//! the issue #1339 acceptance criterion requires they survive
//! restart so the UI can show / dismiss orphans. Decisions on
//! orphan rows update the DB but cannot resume a parked future
//! across processes — no side effect can fire across launches, so
//! the security invariant is preserved without auto-purging.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;

use super::store;
use super::types::{ApprovalDecision, GateOutcome, PendingApproval};

/// How long the gate will park a future before timing out and
/// returning `Deny`. 10 minutes matches the default `expires_at`
/// written into the persisted row.
const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(60 * 10);

static GLOBAL_GATE: OnceLock<Arc<ApprovalGate>> = OnceLock::new();

/// Coordinator for pending approvals.
pub struct ApprovalGate {
    config: Config,
    session_id: String,
    ttl: Duration,
    waiters: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
    always_allowlist: Mutex<HashSet<String>>,
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
        Self {
            config,
            session_id,
            ttl,
            waiters: Mutex::new(HashMap::new()),
            always_allowlist: Mutex::new(HashSet::new()),
        }
    }

    /// Intercept a tool call. Blocks until the user decides or the
    /// TTL elapses (timeout → `Deny`).
    pub async fn intercept(
        &self,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
    ) -> GateOutcome {
        // Session-scoped allowlist shortcut — set by prior
        // ApproveAlwaysForTool decisions in this launch.
        {
            let list = self.always_allowlist.lock();
            if list.contains(tool_name) {
                tracing::debug!(
                    tool = tool_name,
                    "[approval::gate] session-allowlist hit, skipping prompt"
                );
                return GateOutcome::Allow;
            }
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let expires_at = Some(now + chrono::Duration::from_std(self.ttl).unwrap_or_default());
        let pending = PendingApproval {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted: args_redacted.clone(),
            session_id: self.session_id.clone(),
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

        if let Err(err) = store::insert_pending(&self.config, &pending) {
            self.evict_waiter(&request_id);
            tracing::error!(
                error = %err,
                tool = tool_name,
                "[approval::gate] failed to persist pending row — failing closed"
            );
            return GateOutcome::Deny {
                reason: format!(
                    "Approval gate could not persist the request — denying for safety: {err}"
                ),
            };
        }

        publish_global(DomainEvent::ApprovalRequested {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted,
            session_id: self.session_id.clone(),
        });

        tracing::info!(
            request_id = %request_id,
            tool = tool_name,
            "[approval::gate] tool call parked, waiting for decision"
        );

        match tokio::time::timeout(self.ttl, rx).await {
            Ok(Ok(decision)) => {
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    decision = decision.as_str(),
                    "[approval::gate] decision received"
                );
                if decision.is_approve() {
                    GateOutcome::Allow
                } else {
                    GateOutcome::Deny {
                        reason: format!("User denied '{tool_name}' execution."),
                    }
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
                GateOutcome::Deny {
                    reason: format!(
                        "Approval channel for '{tool_name}' closed before a decision was made."
                    ),
                }
            }
            Err(_elapsed) => {
                self.evict_waiter(&request_id);
                let _ = store::decide(&self.config, &request_id, ApprovalDecision::Deny);
                tracing::warn!(
                    request_id = %request_id,
                    tool = tool_name,
                    ttl_secs = self.ttl.as_secs(),
                    "[approval::gate] approval timed out, denying"
                );
                GateOutcome::Deny {
                    reason: format!(
                        "Approval for '{tool_name}' timed out after {}s.",
                        self.ttl.as_secs()
                    ),
                }
            }
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
            if decision == ApprovalDecision::ApproveAlwaysForTool {
                let mut list = self.always_allowlist.lock();
                list.insert(row.tool_name.clone());
            }
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
        let session = format!("test-session-{}", uuid::Uuid::new_v4());
        let gate = ApprovalGate::new(config, session, Duration::from_millis(500));
        (gate, dir)
    }

    #[tokio::test]
    async fn approve_once_returns_allow() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.intercept("composio", "send slack", serde_json::json!({}))
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
            g.intercept("pushover", "send push", serde_json::json!({}))
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
    async fn approve_always_for_tool_short_circuits_future_calls() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let first = tokio::spawn(async move {
            g.intercept("composio", "first", serde_json::json!({}))
                .await
        });
        let pending = loop {
            if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        gate.decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForTool)
            .unwrap();
        assert!(matches!(first.await.unwrap(), GateOutcome::Allow));

        // Second call to the same tool must NOT block — allowlist hit.
        let outcome = gate
            .intercept("composio", "second", serde_json::json!({}))
            .await;
        assert!(matches!(outcome, GateOutcome::Allow));
    }

    #[tokio::test]
    async fn timeout_returns_deny() {
        let (gate, _dir) = test_gate(); // TTL = 500ms
        let gate = Arc::new(gate);
        let outcome = gate
            .intercept("composio", "timed out", serde_json::json!({}))
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
}
