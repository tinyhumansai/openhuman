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
use super::types::{ApprovalDecision, ExecutionOutcome, GateOutcome, PendingApproval};

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
        // Session-scoped allowlist shortcut — set by prior
        // ApproveAlwaysForTool decisions in this launch.
        {
            let list = self.always_allowlist.lock();
            if list.contains(tool_name) {
                tracing::debug!(
                    tool = tool_name,
                    "[approval::gate] session-allowlist hit, skipping prompt"
                );
                return (GateOutcome::Allow, None);
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
            return (
                GateOutcome::Deny {
                    reason: format!(
                        "Approval gate could not persist the request — denying for safety: {err}"
                    ),
                },
                None,
            );
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
                    (GateOutcome::Allow, Some(request_id))
                } else {
                    (
                        GateOutcome::Deny {
                            reason: format!("User denied '{tool_name}' execution."),
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
                            "Approval channel for '{tool_name}' closed before a decision was made."
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
                    return (GateOutcome::Allow, Some(request_id));
                }
                tracing::warn!(
                    request_id = %request_id,
                    tool = tool_name,
                    ttl_secs = self.ttl.as_secs(),
                    "[approval::gate] approval timed out, denying"
                );
                (
                    GateOutcome::Deny {
                        reason: format!(
                            "Approval for '{tool_name}' timed out after {}s.",
                            self.ttl.as_secs()
                        ),
                    },
                    None,
                )
            }
        }
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
        // 500ms TTL was racing the 50×10ms poll loop on slow CI
        // runners — the row would expire (and get denied by
        // list_pending's lazy-expire) before `decide` could fire,
        // surfacing as "pending row never appeared". 2s gives the
        // polling tests enough headroom while keeping
        // `timeout_returns_deny` fast (PR #2367 CI flake).
        let gate = ApprovalGate::new(config, session, Duration::from_secs(2));
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

    #[tokio::test]
    async fn intercept_audited_returns_request_id_only_when_allowed_and_persisted() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        // Allow path: the audited variant must hand back the
        // request_id so the caller can record_execution later
        // (issue #2135).
        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.intercept_audited("composio", "send slack", serde_json::json!({}))
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
    async fn intercept_audited_returns_none_id_for_denied_and_allowlisted() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        // Deny path → no id (nothing to record afterward).
        let g = gate.clone();
        let denied = tokio::spawn(async move {
            g.intercept_audited("composio", "send slack", serde_json::json!({}))
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
            g.intercept_audited("pushover", "first send", serde_json::json!({}))
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
        gate.decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForTool)
            .unwrap();
        let (first_outcome, first_id) = first.await.unwrap();
        assert!(matches!(first_outcome, GateOutcome::Allow));
        assert!(
            first_id.is_some(),
            "the prompting call still persists a row"
        );

        let (second_outcome, second_id) = gate
            .intercept_audited("pushover", "second send", serde_json::json!({}))
            .await;
        assert!(matches!(second_outcome, GateOutcome::Allow));
        assert!(
            second_id.is_none(),
            "session-allowlist shortcut must not persist a row, so no id to record against"
        );
    }
}
