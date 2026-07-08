//! Attention queue: fold the cross-domain "needs you" signals for the
//! orchestration hub into one flat, priority-ordered list.
//!
//! The TinyPlace Orchestration tab renders this as a single "Needs you" zone so
//! a user running many agent instances sees, in one place, everything blocked on
//! them: tool calls parked on the approval gate, agent runs awaiting input, and
//! instances with unread inbound messages. This module owns only the pure
//! aggregation + ordering — the handler in [`super::schemas`] gathers the raw
//! signals from each source domain and hands them here.
//!
//! Keeping this out of `ops.rs` (already large) preserves the single-file
//! single-responsibility rule and lets the whole assembly be unit-tested without
//! a live approval gate, command center, or store.

use serde::Serialize;

/// The kind of attention signal, in descending urgency. Serialized kebab-case
/// so the renderer can key an icon/tone off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AttentionKind {
    /// A tool call is parked on the approval gate awaiting the user's decision.
    Approval,
    /// An agent run is blocked awaiting user input (command-center NeedsInput).
    NeedsInput,
    /// An instance has unread inbound messages.
    Unread,
}

impl AttentionKind {
    /// Sort weight — lower is more urgent (rendered first). Approvals are
    /// TTL-bound (they auto-deny), so they lead; blocked runs next; unread last.
    fn priority(self) -> u8 {
        match self {
            AttentionKind::Approval => 0,
            AttentionKind::NeedsInput => 1,
            AttentionKind::Unread => 2,
        }
    }
}

/// What the renderer should do when the user acts on an item. Tagged so the
/// frontend can `switch` on `type` and carry exactly the id it needs.
///
/// `rename_all` only renames the variant tag; `rename_all_fields` is required to
/// carry the struct-variant fields to camelCase (`requestId`, `threadId`, …) so
/// the wire matches the `AttentionAction` union the TS client declares — without
/// it the ids serialize snake_case and the renderer router reads `undefined`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum AttentionAction {
    /// Open the approval decision surface for `requestId`.
    Approval { request_id: String },
    /// Open a conversation thread transcript.
    OpenThread { thread_id: String },
    /// Open an agent run (no thread linked yet).
    OpenRun { run_id: String },
    /// Open an orchestration chat window.
    OpenSession { session_id: String },
}

/// One actionable row in the attention queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttentionItem {
    /// Stable key for the renderer list (`<kind>:<source-id>`).
    pub id: String,
    pub kind: AttentionKind,
    /// The instance/source this concerns (request id / run id / session id).
    pub instance_id: String,
    /// Short label (tool name / agent display name / session label).
    pub title: String,
    /// One-line, PII-safe detail (approval/run summary). `None` for unread —
    /// the frontend localizes a count instead (`count`), so no English leaks
    /// into the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Unread message count — set only for the `unread` kind so the renderer can
    /// localize "N unread" rather than the backend shipping English.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    pub action: AttentionAction,
    /// RFC3339 creation/activity time when known — used for the newest-first
    /// tie-break within a kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Per-kind + total counts so the UI can badge the zone without re-counting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttentionCounts {
    pub total: usize,
    pub approvals: usize,
    pub needs_input: usize,
    pub unread: usize,
}

/// The assembled attention queue returned by `orchestration_attention`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttentionQueue {
    pub items: Vec<AttentionItem>,
    pub counts: AttentionCounts,
}

// ── Neutral input signals ───────────────────────────────────────────────────
//
// Decoupled from the source domains' own types (`PendingApproval`,
// `AgentWorkRow`, `SessionSummary`) so the assembly is trivially testable and
// this module never depends on those crates' internals.

/// A tool call parked on the approval gate.
pub(crate) struct ApprovalSignal {
    pub request_id: String,
    pub tool_name: String,
    pub action_summary: String,
    pub created_at: Option<String>,
}

/// An agent run in the command-center `NeedsInput` bucket.
pub(crate) struct NeedsInputSignal {
    pub run_id: String,
    /// Display name, falling back to the run id at the call site.
    pub title: String,
    pub summary: Option<String>,
    /// Preferred deep-link target: a worker/parent thread, when the run has one.
    pub thread_id: Option<String>,
    pub updated_at: Option<String>,
}

/// An orchestration instance with unread inbound messages.
pub(crate) struct UnreadSignal {
    pub session_id: String,
    pub label: Option<String>,
    pub unread: i64,
    pub last_message_at: Option<String>,
}

/// A REMOTE agent session parked on a tool-approval decision. Phase 1 persists
/// `status.state = "waiting_approval"` on the session (plus the prompt in
/// `current_detail` and the in-flight `active_call_id`) when a v2
/// `approval_request` event arrives from a peer harness. Unlike [`ApprovalSignal`]
/// — which is a call parked on THIS core's local approval gate — this is a
/// remote instance blocked on the user; acting on it opens the orchestration
/// chat window so the user can steer the reply.
pub(crate) struct RemoteApprovalSignal {
    pub session_id: String,
    /// Human-friendly session label; falls back to the session id at the builder.
    pub label: Option<String>,
    /// The approval prompt / current-activity line (PII-safe summary text).
    pub detail: Option<String>,
    /// The in-flight tool call id awaiting the decision, when known. Carried for
    /// parity with the persisted run-state; not currently surfaced on the wire.
    pub active_call_id: Option<String>,
    pub created_at: Option<String>,
}

// ── Builders ────────────────────────────────────────────────────────────────

fn approval_item(sig: ApprovalSignal) -> AttentionItem {
    AttentionItem {
        id: format!("approval:{}", sig.request_id),
        kind: AttentionKind::Approval,
        instance_id: sig.request_id.clone(),
        title: sig.tool_name,
        summary: Some(sig.action_summary),
        count: None,
        action: AttentionAction::Approval {
            request_id: sig.request_id,
        },
        created_at: sig.created_at,
    }
}

fn needs_input_item(sig: NeedsInputSignal) -> AttentionItem {
    // Deep-link to the thread when one exists, else to the run itself.
    let action = match sig.thread_id {
        Some(thread_id) => AttentionAction::OpenThread { thread_id },
        None => AttentionAction::OpenRun {
            run_id: sig.run_id.clone(),
        },
    };
    AttentionItem {
        id: format!("needs-input:{}", sig.run_id),
        kind: AttentionKind::NeedsInput,
        instance_id: sig.run_id,
        title: sig.title,
        summary: sig.summary,
        count: None,
        action,
        created_at: sig.updated_at,
    }
}

fn unread_item(sig: UnreadSignal) -> AttentionItem {
    let title = sig.label.unwrap_or_else(|| sig.session_id.clone());
    AttentionItem {
        id: format!("unread:{}", sig.session_id),
        kind: AttentionKind::Unread,
        title,
        summary: None,
        count: Some(sig.unread),
        action: AttentionAction::OpenSession {
            session_id: sig.session_id.clone(),
        },
        instance_id: sig.session_id,
        created_at: sig.last_message_at,
    }
}

fn remote_approval_item(sig: RemoteApprovalSignal) -> AttentionItem {
    let title = sig.label.unwrap_or_else(|| sig.session_id.clone());
    // Namespaced apart from local approvals (`approval:<request_id>`) so a session
    // id can never collide with a request id in the renderer's list keys, even
    // though both surface as the same `Approval` kind.
    AttentionItem {
        id: format!("remote-approval:{}", sig.session_id),
        kind: AttentionKind::Approval,
        title,
        summary: sig.detail,
        count: None,
        action: AttentionAction::OpenSession {
            session_id: sig.session_id.clone(),
        },
        instance_id: sig.session_id,
        created_at: sig.created_at,
    }
}

// ── Assembly ────────────────────────────────────────────────────────────────

/// Fold the four signal sources into one ordered queue. Unread signals with a
/// zero count are dropped (nothing to surface). Ordering is by kind urgency,
/// then newest-first within a kind, then id for a stable total order.
///
/// `remote_approvals` are peer sessions parked on an approval decision (Phase 1's
/// persisted `waiting_approval` run-state). They share the `Approval` kind with
/// the local-gate `approvals`, so they lead the queue and are counted under
/// `counts.approvals` — they ARE approvals, just remote ones.
pub(crate) fn assemble_attention(
    approvals: Vec<ApprovalSignal>,
    needs_input: Vec<NeedsInputSignal>,
    unread: Vec<UnreadSignal>,
    remote_approvals: Vec<RemoteApprovalSignal>,
) -> AttentionQueue {
    let mut items: Vec<AttentionItem> = Vec::new();
    items.extend(approvals.into_iter().map(approval_item));
    items.extend(remote_approvals.into_iter().map(remote_approval_item));
    items.extend(needs_input.into_iter().map(needs_input_item));
    items.extend(unread.into_iter().filter(|s| s.unread > 0).map(unread_item));

    items.sort_by(|a, b| {
        a.kind
            .priority()
            .cmp(&b.kind.priority())
            // Newest-first within a kind: reverse-compare the RFC3339 strings
            // (lexical order matches chronological order). A missing timestamp
            // sorts last.
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });

    let counts = AttentionCounts {
        approvals: items
            .iter()
            .filter(|i| i.kind == AttentionKind::Approval)
            .count(),
        needs_input: items
            .iter()
            .filter(|i| i.kind == AttentionKind::NeedsInput)
            .count(),
        unread: items
            .iter()
            .filter(|i| i.kind == AttentionKind::Unread)
            .count(),
        total: items.len(),
    };

    AttentionQueue { items, counts }
}

/// Map pending approvals into neutral signals, stamping the creation time as
/// RFC3339 for the newest-first sort.
pub(crate) fn approval_signals(
    pending: Vec<crate::openhuman::approval::types::PendingApproval>,
) -> Vec<ApprovalSignal> {
    pending
        .into_iter()
        .map(|p| ApprovalSignal {
            request_id: p.request_id,
            tool_name: p.tool_name,
            action_summary: p.action_summary,
            created_at: Some(p.created_at.to_rfc3339()),
        })
        .collect()
}

/// Map a command-center view's `NeedsInput` bucket into neutral signals. Pure —
/// the handler passes the already-fetched view. A run deep-links to its worker
/// thread (then parent thread); its display name falls back to the run id.
pub(crate) fn needs_input_from_command_center(
    view: crate::openhuman::agent_orchestration::command_center::CommandCenterView,
) -> Vec<NeedsInputSignal> {
    use crate::openhuman::agent_orchestration::command_center::AgentWorkBucket;
    view.groups
        .into_iter()
        .find(|g| g.bucket == AgentWorkBucket::NeedsInput)
        .map(|group| {
            group
                .rows
                .into_iter()
                .map(|row| NeedsInputSignal {
                    title: row.display_name.unwrap_or_else(|| row.run_id.clone()),
                    thread_id: row.worker_thread_id.or(row.parent_thread_id),
                    summary: row.summary,
                    updated_at: Some(row.updated_at),
                    run_id: row.run_id,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Map orchestration sessions into remote-approval signals — keeping only those
/// whose persisted v2 run-state is `waiting_approval` (Phase 1 stamps this when a
/// peer harness emits an `approval_request`). Pure: the handler passes the
/// already-fetched session list. `detail` carries the PII-safe approval prompt
/// (`current_detail`); the deep link opens the session window (`last_message_at`
/// stamps the newest-first tie-break).
pub(crate) fn remote_approval_signals(
    sessions: Vec<super::types::OrchestrationSession>,
) -> Vec<RemoteApprovalSignal> {
    sessions
        .into_iter()
        .filter(|s| s.status_state.as_deref() == Some("waiting_approval"))
        .map(|s| RemoteApprovalSignal {
            session_id: s.session_id,
            label: s.label,
            detail: s.current_detail,
            active_call_id: s.active_call_id,
            created_at: Some(s.last_message_at),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(id: &str, at: &str) -> ApprovalSignal {
        ApprovalSignal {
            request_id: id.into(),
            tool_name: "shell".into(),
            action_summary: "run a command".into(),
            created_at: Some(at.into()),
        }
    }

    #[test]
    fn empty_sources_yield_empty_queue_with_zero_counts() {
        let q = assemble_attention(vec![], vec![], vec![], vec![]);
        assert!(q.items.is_empty());
        assert_eq!(
            q.counts,
            AttentionCounts {
                total: 0,
                approvals: 0,
                needs_input: 0,
                unread: 0
            }
        );
    }

    #[test]
    fn kinds_order_approval_then_needs_input_then_unread() {
        let q = assemble_attention(
            vec![approval("r1", "2026-07-06T10:00:00Z")],
            vec![NeedsInputSignal {
                run_id: "run-1".into(),
                title: "researcher".into(),
                summary: Some("blocked on a question".into()),
                thread_id: Some("thread-9".into()),
                updated_at: Some("2026-07-06T11:00:00Z".into()),
            }],
            vec![UnreadSignal {
                session_id: "h-1".into(),
                label: Some("Claude · repo audit".into()),
                unread: 3,
                last_message_at: Some("2026-07-06T12:00:00Z".into()),
            }],
            vec![],
        );
        let kinds: Vec<AttentionKind> = q.items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AttentionKind::Approval,
                AttentionKind::NeedsInput,
                AttentionKind::Unread
            ]
        );
        assert_eq!(
            q.counts,
            AttentionCounts {
                total: 3,
                approvals: 1,
                needs_input: 1,
                unread: 1
            }
        );
    }

    #[test]
    fn newest_first_within_a_kind() {
        let q = assemble_attention(
            vec![
                approval("old", "2026-07-06T09:00:00Z"),
                approval("new", "2026-07-06T12:00:00Z"),
                approval("mid", "2026-07-06T10:30:00Z"),
            ],
            vec![],
            vec![],
            vec![],
        );
        let ids: Vec<&str> = q.items.iter().map(|i| i.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[test]
    fn zero_unread_is_dropped() {
        let q = assemble_attention(
            vec![],
            vec![],
            vec![
                UnreadSignal {
                    session_id: "quiet".into(),
                    label: None,
                    unread: 0,
                    last_message_at: None,
                },
                UnreadSignal {
                    session_id: "loud".into(),
                    label: None,
                    unread: 5,
                    last_message_at: Some("2026-07-06T12:00:00Z".into()),
                },
            ],
            vec![],
        );
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].instance_id, "loud");
        assert_eq!(q.items[0].count, Some(5));
        assert_eq!(q.items[0].summary, None);
    }

    #[test]
    fn item_shapes_carry_correct_ids_and_actions() {
        let q = assemble_attention(
            vec![approval("req-7", "2026-07-06T10:00:00Z")],
            vec![
                // Thread present → OpenThread.
                NeedsInputSignal {
                    run_id: "run-a".into(),
                    title: "agent-a".into(),
                    summary: None,
                    thread_id: Some("t-1".into()),
                    updated_at: None,
                },
                // No thread → OpenRun fallback.
                NeedsInputSignal {
                    run_id: "run-b".into(),
                    title: "agent-b".into(),
                    summary: None,
                    thread_id: None,
                    updated_at: None,
                },
            ],
            vec![UnreadSignal {
                session_id: "sess-1".into(),
                label: Some("Codex".into()),
                unread: 2,
                last_message_at: None,
            }],
            vec![],
        );
        let by_id = |id: &str| q.items.iter().find(|i| i.id == id).unwrap().clone();

        let appr = by_id("approval:req-7");
        assert_eq!(
            appr.action,
            AttentionAction::Approval {
                request_id: "req-7".into()
            }
        );

        let run_a = by_id("needs-input:run-a");
        assert_eq!(
            run_a.action,
            AttentionAction::OpenThread {
                thread_id: "t-1".into()
            }
        );
        let run_b = by_id("needs-input:run-b");
        assert_eq!(
            run_b.action,
            AttentionAction::OpenRun {
                run_id: "run-b".into()
            }
        );

        let unread = by_id("unread:sess-1");
        assert_eq!(
            unread.action,
            AttentionAction::OpenSession {
                session_id: "sess-1".into()
            }
        );
        assert_eq!(unread.title, "Codex");
    }

    #[test]
    fn action_serializes_tagged_with_camelcase_id_fields() {
        // The TS `AttentionAction` union reads `requestId`/`threadId`/`runId`/
        // `sessionId`; the wire must match or the renderer router gets `undefined`.
        let cases = [
            (
                AttentionAction::Approval {
                    request_id: "r".into(),
                },
                "approval",
                "requestId",
            ),
            (
                AttentionAction::OpenThread {
                    thread_id: "t".into(),
                },
                "open-thread",
                "threadId",
            ),
            (
                AttentionAction::OpenRun { run_id: "n".into() },
                "open-run",
                "runId",
            ),
            (
                AttentionAction::OpenSession {
                    session_id: "s".into(),
                },
                "open-session",
                "sessionId",
            ),
        ];
        for (action, tag, id_field) in cases {
            let v = serde_json::to_value(&action).unwrap();
            assert_eq!(v.get("type").and_then(|x| x.as_str()), Some(tag));
            assert!(
                v.get(id_field).is_some(),
                "action {tag} must expose camelCase id field {id_field}, got {v}"
            );
        }
    }

    #[test]
    fn unread_title_falls_back_to_session_id_when_unlabeled() {
        let q = assemble_attention(
            vec![],
            vec![],
            vec![UnreadSignal {
                session_id: "h-xyz".into(),
                label: None,
                unread: 1,
                last_message_at: None,
            }],
            vec![],
        );
        assert_eq!(q.items[0].title, "h-xyz");
    }

    // ── command-center mapping ──────────────────────────────────────────────

    use crate::openhuman::agent_orchestration::command_center::{
        AgentWorkBucket, AgentWorkRow, CommandCenterGroup, CommandCenterView,
    };

    fn work_row(run_id: &str, bucket: AgentWorkBucket) -> AgentWorkRow {
        AgentWorkRow {
            run_id: run_id.into(),
            kind: "subagent".into(),
            agent_id: None,
            display_name: None,
            bucket,
            status: "awaiting_user".into(),
            parent_thread_id: None,
            worker_thread_id: None,
            summary: None,
            error: None,
            started_at: "2026-07-06T10:00:00Z".into(),
            updated_at: "2026-07-06T10:05:00Z".into(),
            elapsed_ms: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            tool_count: 0,
        }
    }

    fn view(groups: Vec<CommandCenterGroup>) -> CommandCenterView {
        let total = groups.iter().map(|g| g.rows.len()).sum();
        CommandCenterView { groups, total }
    }

    #[test]
    fn command_center_maps_only_the_needs_input_bucket() {
        // A NeedsInput run with a worker thread + display name, plus a Working
        // run that must be ignored.
        let blocked = AgentWorkRow {
            display_name: Some("researcher".into()),
            worker_thread_id: Some("worker-1".into()),
            parent_thread_id: Some("parent-1".into()),
            summary: Some("needs a decision".into()),
            ..work_row("run-blocked", AgentWorkBucket::NeedsInput)
        };
        let v = view(vec![
            CommandCenterGroup {
                bucket: AgentWorkBucket::NeedsInput,
                count: 1,
                rows: vec![blocked],
            },
            CommandCenterGroup {
                bucket: AgentWorkBucket::Working,
                count: 1,
                rows: vec![work_row("run-working", AgentWorkBucket::Working)],
            },
        ]);

        let signals = needs_input_from_command_center(v);
        assert_eq!(signals.len(), 1, "only the NeedsInput bucket maps");
        let s = &signals[0];
        assert_eq!(s.run_id, "run-blocked");
        assert_eq!(s.title, "researcher");
        // Worker thread wins over parent thread for the deep link.
        assert_eq!(s.thread_id.as_deref(), Some("worker-1"));
        assert_eq!(s.summary.as_deref(), Some("needs a decision"));
    }

    #[test]
    fn command_center_falls_back_to_run_id_and_parent_thread() {
        // No display name → run id; no worker thread → parent thread.
        let row = AgentWorkRow {
            parent_thread_id: Some("parent-9".into()),
            ..work_row("run-x", AgentWorkBucket::NeedsInput)
        };
        let signals = needs_input_from_command_center(view(vec![CommandCenterGroup {
            bucket: AgentWorkBucket::NeedsInput,
            count: 1,
            rows: vec![row],
        }]));
        assert_eq!(signals[0].title, "run-x");
        assert_eq!(signals[0].thread_id.as_deref(), Some("parent-9"));
    }

    #[test]
    fn approval_signals_map_fields_and_stamp_created_at() {
        use crate::openhuman::approval::types::PendingApproval;
        let p = PendingApproval::new("r1", "shell", "run ls", serde_json::json!({}), None);
        let sigs = approval_signals(vec![p]);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].request_id, "r1");
        assert_eq!(sigs[0].tool_name, "shell");
        assert_eq!(sigs[0].action_summary, "run ls");
        assert!(sigs[0].created_at.is_some(), "created_at is stamped");
    }

    #[test]
    fn command_center_empty_when_no_needs_input_group() {
        let signals = needs_input_from_command_center(view(vec![CommandCenterGroup {
            bucket: AgentWorkBucket::Completed,
            count: 0,
            rows: vec![],
        }]));
        assert!(signals.is_empty());
    }

    // ── remote-approval mapping ─────────────────────────────────────────────

    use crate::openhuman::orchestration::types::OrchestrationSession;

    fn session(id: &str, state: Option<&str>) -> OrchestrationSession {
        OrchestrationSession {
            session_id: id.into(),
            agent_id: "@peer".into(),
            source: "claude".into(),
            last_seq: 1,
            created_at: "2026-07-06T00:00:00Z".into(),
            last_message_at: "2026-07-06T12:00:00Z".into(),
            status_state: state.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn remote_approval_signals_keep_only_waiting_approval_sessions() {
        let waiting = OrchestrationSession {
            label: Some("Claude · deploy".into()),
            current_detail: Some("approve `rm -rf build/`".into()),
            active_call_id: Some("call-9".into()),
            ..session("h-waiting", Some("waiting_approval"))
        };
        let signals = remote_approval_signals(vec![
            waiting,
            session("h-running", Some("running")),
            session("h-idle", Some("idle")),
            session("h-legacy", None),
        ]);
        assert_eq!(signals.len(), 1, "only the waiting_approval session maps");
        let s = &signals[0];
        assert_eq!(s.session_id, "h-waiting");
        assert_eq!(s.label.as_deref(), Some("Claude · deploy"));
        assert_eq!(s.detail.as_deref(), Some("approve `rm -rf build/`"));
        assert_eq!(s.active_call_id.as_deref(), Some("call-9"));
        assert_eq!(s.created_at.as_deref(), Some("2026-07-06T12:00:00Z"));
    }

    fn remote(session_id: &str, at: &str) -> RemoteApprovalSignal {
        RemoteApprovalSignal {
            session_id: session_id.into(),
            label: Some("Claude · deploy".into()),
            detail: Some("approve `rm -rf build/`".into()),
            active_call_id: Some("call-9".into()),
            created_at: Some(at.into()),
        }
    }

    #[test]
    fn remote_approval_surfaces_as_approval_kind_ahead_of_other_kinds() {
        let q = assemble_attention(
            vec![],
            vec![NeedsInputSignal {
                run_id: "run-1".into(),
                title: "researcher".into(),
                summary: None,
                thread_id: Some("t-1".into()),
                updated_at: Some("2026-07-06T11:00:00Z".into()),
            }],
            vec![UnreadSignal {
                session_id: "h-2".into(),
                label: None,
                unread: 2,
                last_message_at: Some("2026-07-06T13:00:00Z".into()),
            }],
            vec![remote("h-remote", "2026-07-06T12:00:00Z")],
        );
        // The remote approval leads (Approval kind, priority 0).
        assert_eq!(q.items[0].kind, AttentionKind::Approval);
        assert_eq!(q.items[0].id, "remote-approval:h-remote");
        assert_eq!(
            q.items[0].action,
            AttentionAction::OpenSession {
                session_id: "h-remote".into()
            }
        );
        assert_eq!(
            q.items[0].summary.as_deref(),
            Some("approve `rm -rf build/`")
        );
        assert_eq!(q.items[0].title, "Claude · deploy");
        // Counted under approvals — a remote approval IS an approval.
        assert_eq!(
            q.counts,
            AttentionCounts {
                total: 3,
                approvals: 1,
                needs_input: 1,
                unread: 1
            }
        );
    }

    #[test]
    fn remote_approval_title_falls_back_to_session_id_when_unlabeled() {
        let q = assemble_attention(
            vec![],
            vec![],
            vec![],
            vec![RemoteApprovalSignal {
                session_id: "h-bare".into(),
                label: None,
                detail: None,
                active_call_id: None,
                created_at: None,
            }],
        );
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].title, "h-bare");
        assert_eq!(q.counts.approvals, 1);
    }

    #[test]
    fn empty_remote_approvals_is_a_no_op() {
        // A queue with only a local approval is unchanged by an empty remote vec.
        let q = assemble_attention(
            vec![approval("r1", "2026-07-06T10:00:00Z")],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].id, "approval:r1");
        assert_eq!(q.counts.approvals, 1);
    }

    #[test]
    fn local_and_remote_approvals_coexist_newest_first() {
        // Both are Approval kind; within the kind they order newest-first, so the
        // later remote approval leads the earlier local one.
        let q = assemble_attention(
            vec![approval("local-old", "2026-07-06T09:00:00Z")],
            vec![],
            vec![],
            vec![remote("remote-new", "2026-07-06T12:00:00Z")],
        );
        assert_eq!(q.counts.approvals, 2);
        let ids: Vec<&str> = q.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["remote-approval:remote-new", "approval:local-old"]
        );
    }
}
