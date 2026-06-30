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
use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::security::POLICY_DENIED_MARKER;

use super::store;
use super::types::{ApprovalDecision, ExecutionOutcome, GateOutcome, PendingApproval};

/// How long the gate will park a future before timing out and
/// returning `Deny`. 10 minutes matches the default `expires_at`
/// written into the persisted row.
const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(60 * 10);

/// Shorter park window for approvals raised mid-call (issue #3513): a
/// live meeting can't idle on a parked tool for the default ten
/// minutes — if nobody approves within two, deny and move on.
const IN_CALL_APPROVAL_TTL: Duration = Duration::from_secs(120);

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

/// In-call meeting context (issue #3513) — set by `agent_meetings::in_call`
/// around the orchestrator turn for a live meeting. When present, a parked
/// approval additionally:
/// - publishes [`DomainEvent::InCallApprovalRequested`] so the meeting bus
///   can speak the approval prompt into the call (`bot:speak`),
/// - registers a meeting → request mapping so a spoken
///   "Hey Tiny, approve" can be routed to [`ApprovalGate::decide`], and
/// - clamps the park window to [`IN_CALL_APPROVAL_TTL`].
#[derive(Clone, Debug)]
pub struct InCallApprovalContext {
    /// Stable per-meeting key (the correlation id, or `"default"`).
    pub meeting_key: String,
    /// Original correlation id, echoed on spoken prompts.
    pub correlation_id: Option<String>,
}

tokio::task_local! {
    pub static APPROVAL_IN_CALL_CONTEXT: InCallApprovalContext;
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

/// Snapshot of the host-aware boot decision the runtime made when it
/// evaluated `OPENHUMAN_APPROVAL_GATE`. Surfaced to the UI banner via
/// `approval_get_gate_state` so the user sees a banner the *first* time
/// they open the app after an override was honored, not only when a
/// connected socket happens to receive the boot-time domain event.
///
/// Set exactly once on boot from `bootstrap_core_runtime`; subsequent
/// reads return the same snapshot for the lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalGateBootState {
    /// True when the gate was installed at boot.
    pub installed: bool,
    /// True when an `OPENHUMAN_APPROVAL_GATE=0` env override was honored
    /// (CLI / Docker host) — the gate is OFF and external_effect tools
    /// run unprompted. UI banners on this state.
    pub disabled_by_env: bool,
    /// True when an `OPENHUMAN_APPROVAL_GATE=0` env override was observed
    /// but suppressed because the host is the Tauri desktop shell. UI
    /// surfaces a softer one-shot info banner so the user knows the
    /// override was rejected.
    pub override_ignored: bool,
    /// Host tag the boot decision keyed off — `tauri-shell` / `cli` /
    /// `docker`. Pinned strings; downstream consumers may switch on this.
    pub host: &'static str,
}

static BOOT_STATE: OnceLock<ApprovalGateBootState> = OnceLock::new();

/// Record the host-aware boot decision so the UI / RPC layer can read it
/// back. Idempotent — only the first call wins, mirroring the gate
/// `OnceLock` install pattern.
pub fn record_boot_state(state: ApprovalGateBootState) {
    let _ = BOOT_STATE.set(state);
}

/// Read the recorded boot state. Returns `None` when `record_boot_state`
/// was never called (e.g. older test paths that bring up the gate
/// directly without going through `bootstrap_core_runtime`); RPC and UI
/// callers treat that as "no banner needed".
pub fn try_boot_state() -> Option<ApprovalGateBootState> {
    BOOT_STATE.get().copied()
}

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
    /// meeting_key → request_id for the approval currently parked on a live
    /// meeting, so a spoken "Hey Tiny, approve" can be routed to a decision
    /// (issue #3513). Same in-memory/session-scoped semantics as
    /// `thread_to_request`.
    meeting_to_request: Mutex<HashMap<String, String>>,
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
            meeting_to_request: Mutex::new(HashMap::new()),
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
        // Origin tells us who scheduled this turn. Entry points (web channel,
        // channel runtime, subconscious, cron, CLI) scope a typed
        // `AgentTurnOrigin` around `run_turn`. Unlabelled callers map to
        // `Unknown`, which is denied — the gate refuses to execute an
        // external_effect tool from an unlabelled call site.
        let origin = turn_origin::current().unwrap_or(AgentTurnOrigin::Unknown);

        // An autonomous goal continuation runs with no user present, so an
        // irreversible external action must never be auto-allowed — not even via
        // the `autonomy.auto_approve` allowlist. Skip the shortcut for that
        // origin and fall through to the parking flow below.
        let is_goal_continuation = matches!(
            &origin,
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::GoalContinuation,
                ..
            }
        );

        // "Always allow" allowlist shortcut — the user's persisted
        // `autonomy.auto_approve` set. Read from the live policy first so a
        // grant made earlier in this session (which writes config + reloads the
        // live policy) takes effect on the very next tool call; fall back to the
        // gate's boot-time config when no live policy is installed (e.g. a CLI
        // invocation that never started a session runtime, or a unit test).
        if !is_goal_continuation && self.tool_is_auto_approved(tool_name) {
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

        // In-call meeting context — set by agent_meetings::in_call around a
        // live-meeting orchestrator turn. Enables the spoken approval
        // channel alongside the thread card (issue #3513).
        let in_call_ctx = APPROVAL_IN_CALL_CONTEXT.try_with(|c| c.clone()).ok();

        // Branch by origin. Web chat parks for an in-app approval; external
        // channel persists an audit row and TTL-denies (no routable approval
        // surface yet); trusted automation (cron, internal-only subconscious)
        // is allowed through unchanged; tainted subconscious — a tick whose
        // memory context contains external-sync chunks — is denied because
        // remote text could otherwise steer it into an external_effect tool;
        // CLI keeps the legacy allow; Unknown fails closed.
        match &origin {
            AgentTurnOrigin::WebChat { .. } => {
                // Fall through to the existing chat-routed parking flow below.
            }
            AgentTurnOrigin::ExternalChannel {
                channel,
                sender,
                reply_target,
                message_id,
            } => {
                tracing::info!(
                    tool = tool_name,
                    channel = %channel,
                    sender = %sender.as_deref().unwrap_or("<unknown>"),
                    reply_target = %reply_target,
                    message_id = %message_id,
                    in_call = in_call_ctx.is_some(),
                    "[approval::gate] external channel turn — persisting audit row and parking"
                );
                // Fall through to the parking flow: a `pending_approvals` row
                // is persisted (audit trail) and the future parks. We do NOT
                // short-circuit to Allow here — remote inputs are untrusted.
                // Without a routable surface the park TTL-denies; with the
                // in-call context set (live meeting, issue #3513) a decision
                // can arrive via the spoken channel (`pending_for_meeting` →
                // `decide`) or the thread card before the (clamped) TTL.
            }
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Cron,
                job_id,
            } => {
                tracing::debug!(
                    tool = tool_name,
                    job_id = %job_id,
                    "[approval::gate] trusted cron automation — allowing without prompt"
                );
                return (GateOutcome::Allow, None);
            }
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Subconscious,
                job_id,
            } => {
                tracing::debug!(
                    tool = tool_name,
                    job_id = %job_id,
                    "[approval::gate] trusted internal subconscious tick — allowing without prompt"
                );
                return (GateOutcome::Allow, None);
            }
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::SubconsciousTainted,
                job_id,
            } => {
                tracing::warn!(
                    tool = tool_name,
                    job_id = %job_id,
                    "[approval::gate] subconscious tick with external-sync memory in context — \
                     rejecting external_effect tool"
                );
                return (
                    GateOutcome::Deny {
                        reason: format!(
                            "{POLICY_DENIED_MARKER} Tool '{tool_name}' rejected: subconscious turn \
                             whose memory context includes external-sync chunks may not run \
                             external_effect tools."
                        ),
                    },
                    None,
                );
            }
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::GoalContinuation,
                job_id,
            } => {
                tracing::debug!(
                    tool = tool_name,
                    job_id = %job_id,
                    "[approval::gate] autonomous goal continuation — external_effect tool parks \
                     (no present user to authorize); TTL-denies without a routable surface"
                );
                // Fall through to the parking flow: an autonomous continuation
                // runs with no user present, so we must NOT auto-allow an
                // irreversible external action. Read/compute tools (not gated
                // here) still make progress on the goal.
            }
            AgentTurnOrigin::Cli => {
                tracing::debug!(
                    tool = tool_name,
                    "[approval::gate] CLI / sub-agent caller — allowing without prompt"
                );
                return (GateOutcome::Allow, None);
            }
            AgentTurnOrigin::Unknown => {
                tracing::warn!(
                    tool = tool_name,
                    "[approval::gate] agent turn has no origin label — refusing to execute \
                     external_effect tool from unlabelled call site"
                );
                return (
                    GateOutcome::Deny {
                        reason: format!(
                            "{POLICY_DENIED_MARKER} Tool '{tool_name}' rejected: agent turn has \
                             no origin label. Refusing external_effect tool from unlabelled call \
                             site."
                        ),
                    },
                    None,
                );
            }
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let expires_at =
            Some(now + chrono::Duration::from_std(self.effective_ttl()).unwrap_or_default());
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
        // Record the meeting → request mapping so a spoken approval reply
        // ("Hey Tiny, approve") can be routed to a decision.
        if let Some(ic) = in_call_ctx.as_ref() {
            self.meeting_to_request
                .lock()
                .insert(ic.meeting_key.clone(), request_id.clone());
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

        // Voice channel (issue #3513): tell the meeting bus to speak the
        // approval prompt into the call.
        if let Some(ic) = in_call_ctx.as_ref() {
            publish_global(DomainEvent::InCallApprovalRequested {
                request_id: request_id.clone(),
                tool_name: tool_name.to_string(),
                action_summary: action_summary.to_string(),
                correlation_id: ic.correlation_id.clone(),
            });
        }

        tracing::info!(
            request_id = %request_id,
            tool = tool_name,
            "[approval::gate] tool call parked, waiting for decision"
        );

        // Live meetings get a clamped park window — see IN_CALL_APPROVAL_TTL.
        // `effective_ttl()` applies the debug-only env override; the in-call
        // clamp is applied on top so a longer override can't extend a live
        // meeting's park window past IN_CALL_APPROVAL_TTL.
        let effective_ttl = if in_call_ctx.is_some() {
            IN_CALL_APPROVAL_TTL.min(self.effective_ttl())
        } else {
            self.effective_ttl()
        };

        let outcome = match tokio::time::timeout(effective_ttl, rx).await {
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
                        ttl_secs = effective_ttl.as_secs(),
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
                        ttl_secs = effective_ttl.as_secs(),
                        "[approval::gate] approval timed out, denying"
                    );
                    (
                        GateOutcome::Deny {
                            reason: format!(
                                "{POLICY_DENIED_MARKER} Approval for '{tool_name}' timed out after \
                                 {}s. Do not re-request the same call this turn; take a different \
                                 approach or stop.",
                                effective_ttl.as_secs()
                            ),
                        },
                        None,
                    )
                }
            }
        };
        // The routing mappings are only needed while parked; clear them on
        // every exit (decision, channel drop, or timeout).
        self.clear_thread(&chat_thread_id);
        self.clear_meeting(&in_call_ctx);
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

    /// The request_id of the approval currently parked on a live meeting, if
    /// any. Used by `agent_meetings::in_call` to route a spoken
    /// "Hey Tiny, approve" to a decision (issue #3513).
    pub fn pending_for_meeting(&self, meeting_key: &str) -> Option<String> {
        self.meeting_to_request.lock().get(meeting_key).cloned()
    }

    /// Drop the thread → request mapping (best-effort; no-op when absent).
    fn clear_thread(&self, thread_id: &Option<String>) {
        if let Some(t) = thread_id {
            self.thread_to_request.lock().remove(t);
        }
    }

    /// Drop the meeting → request mapping (best-effort; no-op when absent).
    fn clear_meeting(&self, ctx: &Option<InCallApprovalContext>) {
        if let Some(ic) = ctx {
            self.meeting_to_request.lock().remove(&ic.meeting_key);
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

    /// A matching web-chat origin for the chat context fixture. Tests
    /// exercising the parking flow scope BOTH task-locals — production
    /// callers in `channels/providers/web` do the same.
    fn web_origin() -> AgentTurnOrigin {
        AgentTurnOrigin::WebChat {
            thread_id: "t-test".into(),
            client_id: "c-test".into(),
        }
    }

    /// An external-channel (live meeting) origin for the in-call fixtures.
    fn meet_origin() -> AgentTurnOrigin {
        AgentTurnOrigin::ExternalChannel {
            channel: "meet".into(),
            sender: None,
            reply_target: "meet-1".into(),
            message_id: "m-1".into(),
        }
    }

    fn in_call_ctx() -> InCallApprovalContext {
        InCallApprovalContext {
            meeting_key: "meet-1".into(),
            correlation_id: Some("meet-1".into()),
        }
    }

    #[tokio::test]
    async fn in_call_voice_approve_resolves_parked_external_channel_approval() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                meet_origin(),
                APPROVAL_IN_CALL_CONTEXT.scope(
                    in_call_ctx(),
                    g.intercept("composio", "create calendar event", serde_json::json!({})),
                ),
            )
            .await
        });

        // The meeting → request mapping is the voice channel's lookup key.
        let mut tries = 0;
        let request_id = loop {
            if let Some(r) = gate.pending_for_meeting("meet-1") {
                break r;
            }
            tries += 1;
            assert!(tries < 50, "meeting mapping never appeared");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        gate.decide(&request_id, ApprovalDecision::ApproveOnce)
            .unwrap();

        let outcome = handle.await.unwrap();
        assert!(matches!(outcome, GateOutcome::Allow));
        assert!(
            gate.pending_for_meeting("meet-1").is_none(),
            "meeting mapping must be cleared once the park resolves"
        );
    }

    #[tokio::test]
    async fn in_call_voice_deny_resolves_parked_approval_with_deny() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                meet_origin(),
                APPROVAL_IN_CALL_CONTEXT.scope(
                    in_call_ctx(),
                    g.intercept("composio", "send email", serde_json::json!({})),
                ),
            )
            .await
        });

        let request_id = loop {
            if let Some(r) = gate.pending_for_meeting("meet-1") {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        gate.decide(&request_id, ApprovalDecision::Deny).unwrap();

        let outcome = handle.await.unwrap();
        match outcome {
            GateOutcome::Deny { reason } => assert!(reason.contains("composio")),
            other => panic!("expected deny, got {other:?}"),
        }
        assert!(gate.pending_for_meeting("meet-1").is_none());
    }

    #[tokio::test]
    async fn external_channel_without_in_call_ctx_has_no_meeting_mapping() {
        // Plain external-channel turns (telegram, discord) must not gain a
        // voice surface: no in-call context → no meeting mapping. Uses the
        // 2s test TTL so the parked future deny-resolves quickly.
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                meet_origin(),
                g.intercept("composio", "send email", serde_json::json!({})),
            )
            .await
        });

        // Wait for the row to park, then confirm no meeting mapping exists.
        loop {
            if !gate.list_pending().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(gate.pending_for_meeting("meet-1").is_none());

        // TTL-deny is the expected terminal state.
        let outcome = handle.await.unwrap();
        assert!(matches!(outcome, GateOutcome::Deny { .. }));
    }

    #[tokio::test]
    async fn approve_once_returns_allow() {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                web_origin(),
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx(),
                    g.intercept("composio", "send slack", serde_json::json!({})),
                ),
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
            turn_origin::with_origin(
                web_origin(),
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx(),
                    g.intercept("pushover", "send push", serde_json::json!({})),
                ),
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
            dir.path().to_path_buf(),
        );

        // An allow-listed tool short-circuits the gate to `Allow` immediately —
        // before any parking — even with a live chat context present, and
        // without persisting a pending row. The shortcut runs regardless of
        // origin (it's the user's persisted "Always allow" allowlist), so we
        // do not need to scope an origin for this case.
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
        let outcome = turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                gate.intercept("composio", "timed out", serde_json::json!({})),
            ),
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

        // Run intercept inside a scoped chat context + matching WebChat
        // origin (as the web channel does in production).
        let g = gate.clone();
        let ctx = ApprovalChatContext {
            thread_id: "thread-42".into(),
            client_id: "client-1".into(),
        };
        let origin = AgentTurnOrigin::WebChat {
            thread_id: "thread-42".into(),
            client_id: "client-1".into(),
        };
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                origin,
                APPROVAL_CHAT_CONTEXT
                    .scope(ctx, g.intercept("shell", "run ls", serde_json::json!({}))),
            )
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

    /// Tests for `effective_ttl` env-override parsing.
    ///
    /// These run serially (they mutate the process env) via the shared
    /// `TEST_ENV_LOCK`; the lock is the same one used by `auto_approve_tool_skips_prompt`
    /// and the live_policy tests so they cannot clobber each other in parallel.
    ///
    /// Guarded on `debug_assertions`: the override is compiled out of release
    /// builds, so this assertion only holds under `cargo test` (debug). The
    /// fallback tests below hold in either build.
    #[cfg(debug_assertions)]
    #[test]
    fn effective_ttl_uses_env_override_when_valid() {
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (gate, _dir) = test_gate(); // boot-time TTL = 2s
        unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "42") };
        assert_eq!(
            gate.effective_ttl(),
            Duration::from_secs(42),
            "valid OPENHUMAN_APPROVAL_TTL_SECS must override boot-time TTL"
        );
        unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
    }

    #[test]
    fn effective_ttl_falls_back_to_boot_ttl_for_garbage_value() {
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (gate, _dir) = test_gate(); // boot-time TTL = 2s
        unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "not-a-number") };
        assert_eq!(
            gate.effective_ttl(),
            Duration::from_secs(2),
            "garbage OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
        );
        unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
    }

    #[test]
    fn effective_ttl_falls_back_to_boot_ttl_when_unset() {
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (gate, _dir) = test_gate(); // boot-time TTL = 2s
        unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
        assert_eq!(
            gate.effective_ttl(),
            Duration::from_secs(2),
            "unset OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
        );
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
    async fn intercept_with_unknown_origin_denies() {
        // Unlabelled call site (no origin scope) maps to `Unknown` and is
        // rejected. This replaces the previous "no chat context → Allow"
        // legacy behaviour: the gate now refuses to execute external_effect
        // tools from unlabelled call sites.
        let (gate, _dir) = test_gate();
        let outcome = gate
            .intercept("shell", "run ls", serde_json::json!({}))
            .await;
        match outcome {
            GateOutcome::Deny { reason } => assert!(reason.contains("origin label")),
            other => panic!("expected deny, got {other:?}"),
        }
        assert!(gate.pending_for_thread("thread-42").is_none());
    }

    #[tokio::test]
    async fn intercept_with_trusted_cron_origin_allows_without_prompt() {
        // Cron jobs the user explicitly authorized run trusted automation;
        // the gate allows without prompt and does not persist a row.
        let (gate, _dir) = test_gate();
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-42".into(),
            source: TrustedAutomationSource::Cron,
        };
        let outcome = turn_origin::with_origin(
            origin,
            gate.intercept("shell", "run ls", serde_json::json!({})),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Allow));
        assert!(
            gate.list_pending().unwrap().is_empty(),
            "trusted cron must not persist a pending row"
        );
    }

    #[tokio::test]
    async fn intercept_with_trusted_subconscious_origin_allows_without_prompt() {
        // Subconscious ticks on internal-only memory are trusted automation
        // and run unprompted (preserves pre-PR behavior for the safe case).
        let (gate, _dir) = test_gate();
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: "subconscious-tick".into(),
            source: TrustedAutomationSource::Subconscious,
        };
        let outcome = turn_origin::with_origin(
            origin,
            gate.intercept("shell", "run ls", serde_json::json!({})),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Allow));
    }

    #[tokio::test]
    async fn intercept_with_subconscious_tainted_origin_denies() {
        // A subconscious tick whose memory context contains external-sync
        // chunks is rejected for external_effect tools — external text in
        // memory could otherwise steer the tick into a tool call.
        let (gate, _dir) = test_gate();
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: "subconscious-tainted".into(),
            source: TrustedAutomationSource::SubconsciousTainted,
        };
        let outcome = turn_origin::with_origin(
            origin,
            gate.intercept("send_email", "send", serde_json::json!({})),
        )
        .await;
        match outcome {
            GateOutcome::Deny { reason } => {
                assert!(reason.contains("external-sync"), "reason was: {reason}")
            }
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn intercept_with_cli_origin_allows_without_prompt() {
        // CLI / one-off internal callers (sub-agent invocations, scripts)
        // are allowed through unprompted — there is no chat surface to
        // park on, and the legacy CLI workflow assumes the operator
        // authorized the invocation.
        let (gate, _dir) = test_gate();
        let outcome = turn_origin::with_origin(
            AgentTurnOrigin::Cli,
            gate.intercept("shell", "run ls", serde_json::json!({})),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Allow));
    }

    #[tokio::test]
    async fn intercept_with_external_channel_origin_persists_and_ttl_denies() {
        // Non-web channel inbound (Telegram / Discord / Slack / etc.):
        // persist an audit row but TTL-deny — there is no channel-routed
        // approval surface yet, and the input is remote-attacker text.
        let (gate, _dir) = test_gate(); // 2s TTL
        let gate = Arc::new(gate);
        let origin = AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: Some("tg-user-1".into()),
            reply_target: "tg-chat-1".into(),
            message_id: "msg-1".into(),
        };

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            turn_origin::with_origin(
                origin,
                g.intercept("shell", "run ls", serde_json::json!({})),
            )
            .await
        });

        // The audit row appears while the future is parked.
        let mut tries = 0;
        loop {
            if !gate.list_pending().unwrap().is_empty() {
                break;
            }
            tries += 1;
            assert!(tries < 50, "audit row never appeared for external channel");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Without a routable channel approval surface, the parked future
        // TTL-denies (2s — matches the test_gate fixture).
        let outcome = handle.await.unwrap();
        match outcome {
            GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
            other => panic!("expected deny, got {other:?}"),
        }
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
            // Scope a chat context + matching WebChat origin *inside* the
            // spawned task — task-locals don't cross `tokio::spawn`, and
            // `intercept` only parks (creates a pending row) for a chat
            // turn whose origin labels it as web-routable.
            turn_origin::with_origin(
                web_origin(),
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx(),
                    g.intercept_audited("composio", "send slack", serde_json::json!({})),
                ),
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
            turn_origin::with_origin(
                web_origin(),
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx(),
                    g.intercept_audited("composio", "send slack", serde_json::json!({})),
                ),
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
            turn_origin::with_origin(
                web_origin(),
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx(),
                    g.intercept_audited("pushover", "first send", serde_json::json!({})),
                ),
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
