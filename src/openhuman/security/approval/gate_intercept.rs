impl ApprovalGate {
    /// Shared core of [`Self::intercept_audited`] and
    /// [`Self::intercept_audited_bounded`]. When `park_bound` is `Some` and
    /// shorter than the effective TTL, the park is capped at it; on that bound
    /// elapsing the park is abandoned cancellation-safely (waiter evicted,
    /// thread routing cleared, `pending_approvals` row left open) and
    /// `*park_bound_elapsed` is set so the bounded caller can render its own
    /// fast-path result instead of a `Deny`.
    async fn intercept_audited_inner(
        &self,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
        park_bound: Option<Duration>,
        park_bound_elapsed: &mut bool,
    ) -> (GateOutcome, Option<String>) {
        // Origin tells us who scheduled this turn. Entry points (web channel,
        // channel runtime, subconscious, cron, CLI) scope a typed
        // `AgentTurnOrigin` around `run_turn`. Unlabelled callers map to
        // `Unknown`, which is denied — the gate refuses to execute an
        // external_effect tool from an unlabelled call site.
        let origin = turn_origin::current().unwrap_or(AgentTurnOrigin::Unknown);
        tracing::debug!(
            tool = tool_name,
            ?origin,
            auto_approve_all = self.is_auto_approve_all_enabled(),
            bypass_auto = matches!(
                &origin,
                AgentTurnOrigin::TrustedAutomation {
                    source: TrustedAutomationSource::GoalContinuation,
                    ..
                } | AgentTurnOrigin::TrustedAutomation {
                    source: TrustedAutomationSource::Workflow {
                        require_approval: true
                    },
                    ..
                }
            ),
            chat_context = APPROVAL_CHAT_CONTEXT.try_with(|c| c.clone()).is_ok(),
            "[approval::gate] evaluating approval request"
        );

        // Per-flow tool trust shortcut (flow-approval-surface, PR2): a prior
        // `ApproveAlwaysForFlow` decision on this exact `(flow_id, tool_name)`
        // pair short-circuits to `Allow` for every future Workflow-origin call
        // of that tool from that flow — including a `require_approval: true`
        // flow and a Supervised-tier `caps.rs::gate_call_for_tier` escalation,
        // both of which otherwise force the park below. The trust is scoped to
        // the *flow*, never the tool alone, so it cannot leak into a different
        // workflow that happens to call the same tool (that stays gated, or
        // uses the separate global `autonomy.auto_approve` allowlist). Checked
        // before any other origin branching so it wins regardless of which
        // arm of the match below would otherwise fire.
        if let AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Workflow { .. },
            job_id: flow_id,
        } = &origin
        {
            match store::is_flow_tool_trusted(&self.config, flow_id, tool_name) {
                Ok(true) => {
                    tracing::debug!(
                        tool = tool_name,
                        flow_id = %flow_id,
                        "[approval::gate] flow_tool_trust hit — auto-allowing without prompt"
                    );
                    return (GateOutcome::Allow, None);
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(
                        tool = tool_name,
                        flow_id = %flow_id,
                        error = %err,
                        "[approval::gate] flow_tool_trust lookup failed — falling through to \
                         normal gating (fail-safe: still gated, not silently allowed)"
                    );
                }
            }
        }

        // An autonomous goal continuation runs with no user present, so an
        // irreversible external action must never be auto-allowed — not even via
        // the `autonomy.auto_approve` allowlist. Skip the shortcut for that
        // origin and fall through to the parking flow below. A workflow run
        // whose flow has `require_approval` set gets the same treatment — the
        // user explicitly asked for every outbound action on that flow to be
        // gated, and a global tool allowlist must not silently override that
        // per-flow choice.
        let bypass_auto_approve_shortcut = matches!(
            &origin,
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::GoalContinuation,
                ..
            } | AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Workflow {
                    require_approval: true
                },
                ..
            }
        );

        // Blanket "auto-approve everything" bypass (opt-in, off by default).
        // Sits ABOVE the origin match below so it prevents parking entirely
        // for every origin except the two that must never be silently
        // allowed: a subconscious tick whose memory context is tainted by
        // external-sync content (indirect prompt injection defense) and an
        // unlabelled call site (fail-closed default). Both are excluded here
        // so they still fall through to the origin match and hit their Deny
        // arms unchanged. This check is independent of — and does not
        // weaken — `is_always_forbidden`, `is_workspace_internal_path`, or
        // `ToolPolicyMiddleware`, which all run inside the tool
        // implementation itself, not the approval gate.
        //
        // Known and accepted: a **remote-origin triage** dispatch is not in the
        // protected set either. Since openhuman#5634 a Composio/webhook payload
        // reaching `triage.escalate` carries
        // `TrustedAutomation { Workflow { require_approval: true } }`, which
        // normally parks and writes a `pending_approvals` row — and with this
        // flag on it is allowed here instead, leaving no approval trail for
        // those dispatches. The gate owner ruled that enabling a blanket
        // "approve everything" switch opts into that globally rather than
        // carving out an exception, because the alternatives either narrow what
        // the flag means for every user who set it or turn it into "approve
        // everything except…". Decision and the options weighed:
        // https://github.com/tinyhumansai/openhuman/issues/5634#issuecomment-5396604125
        //
        // `auto_approve_all_allows_a_remote_triage_dispatch_without_an_audit_row`
        // below pins that outcome, so a change to this exclusion list has to
        // confront the decision rather than discover it.
        let auto_all = self.is_auto_approve_all_enabled()
            && !matches!(
                &origin,
                AgentTurnOrigin::TrustedAutomation {
                    source: TrustedAutomationSource::SubconsciousTainted,
                    ..
                } | AgentTurnOrigin::Unknown
            );

        if auto_all {
            // `origin_class` is the sanitized variant label (no thread/client
            // ids, channel sender, reply target, or message id) — safe at
            // `info`. The full `?origin` (with those identifiers) is still
            // available at `debug` for local troubleshooting.
            tracing::info!(
                tool = tool_name,
                origin_class = %origin.class(),
                auto_approved = true,
                "[approval::gate] auto_approve_all enabled — auto-approving without prompt"
            );
            tracing::debug!(
                tool = tool_name,
                origin = ?origin,
                "[approval::gate] auto_approve_all full origin (debug-only)"
            );
            return (GateOutcome::Allow, None);
        }

        // "Always allow" allowlist shortcut — the user's persisted
        // `autonomy.auto_approve` set. Read from the live policy first so a
        // grant made earlier in this session (which writes config + reloads the
        // live policy) takes effect on the very next tool call; fall back to the
        // gate's boot-time config when no live policy is installed (e.g. a CLI
        // invocation that never started a session runtime, or a unit test).
        if !bypass_auto_approve_shortcut && self.tool_is_auto_approved(tool_name) {
            tracing::debug!(
                tool = tool_name,
                "[approval::gate] auto_approve allowlist hit, skipping prompt"
            );
            return (GateOutcome::Allow, None);
        }

        // Chat context (thread/client id) for routing the yes/no reply — set by
        // the web channel around the agent run; absent for non-chat callers.
        //
        // Fallback (#5499): when the task-local is absent but the turn is
        // `WebChat`, route via the thread/client the origin itself carries. The
        // web channel scopes `APPROVAL_CHAT_CONTEXT` and builds the `WebChat`
        // origin from the *same* thread_id/client_id (`web_chat::start_chat`),
        // so the two are identical whenever both are present. They diverge only
        // when a turn is carried across a `tokio::spawn` boundary that
        // propagates the origin but not the approval context — most importantly
        // an async-delegated sub-agent (`spawn_async_subagent`, reached when the
        // orchestrator routes "remind me…" to `scheduler_agent`): the origin
        // travels but this task-local does not. Without the fallback the gate
        // parks with `thread_id: None`, the web-channel surface drops the
        // `ApprovalRequested` event ("thread/client absent — NOT surfacing"),
        // and the park silently TTL-denies — so a `cron_add` scheduled from a
        // chat turn never completes.
        let chat_ctx = APPROVAL_CHAT_CONTEXT.try_with(|c| c.clone()).ok();
        let origin_chat_route = match &origin {
            AgentTurnOrigin::WebChat {
                thread_id,
                client_id,
                ..
            } => Some((thread_id.clone(), client_id.clone())),
            _ => None,
        };
        if chat_ctx.is_none() && origin_chat_route.is_some() {
            tracing::debug!(
                tool = tool_name,
                "[approval::gate] APPROVAL_CHAT_CONTEXT absent on a WebChat turn — routing the \
                 approval via the origin's thread/client (async-delegated sub-agent path, #5499)"
            );
        }
        let chat_thread_id = chat_ctx
            .as_ref()
            .map(|c| c.thread_id.clone())
            .or_else(|| origin_chat_route.as_ref().map(|(t, _)| t.clone()));
        let chat_client_id = chat_ctx
            .as_ref()
            .map(|c| c.client_id.clone())
            .or_else(|| origin_chat_route.as_ref().map(|(_, c)| c.clone()));

        // Copilot-streaming context — set by `flows::ops::flows_build` around
        // the streaming `run_single` call. Presence alone clamps the park
        // window to `COPILOT_APPROVAL_TTL`; see that task-local's doc.
        let copilot_stream = APPROVAL_COPILOT_STREAM_CONTEXT.try_with(|_| ()).is_ok();

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
                    "[approval::gate] external channel turn — persisting audit row and parking"
                );
                // Fall through to the parking flow: a `pending_approvals` row
                // is persisted (audit trail) and the future parks. We do NOT
                // short-circuit to Allow here — remote inputs are untrusted.
                // Without a routable surface the park TTL-denies; a decision
                // can still arrive via the thread card before the TTL.
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
            AgentTurnOrigin::TrustedAutomation {
                source:
                    TrustedAutomationSource::Workflow {
                        require_approval: false,
                    },
                job_id,
            } => {
                tracing::debug!(
                    tool = tool_name,
                    flow_id = %job_id,
                    "[approval::gate] trusted workflow automation — pre-declared action, \
                     allowing without prompt"
                );
                return (GateOutcome::Allow, None);
            }
            AgentTurnOrigin::TrustedAutomation {
                source:
                    TrustedAutomationSource::Workflow {
                        require_approval: true,
                    },
                job_id,
            } => {
                tracing::info!(
                    tool = tool_name,
                    flow_id = %job_id,
                    "[approval::gate] workflow run has require_approval enabled — parking for \
                     HITL review instead of auto-allowing the trust root"
                );
                // Fall through to the parking flow (same shape as
                // GoalContinuation): persists a `pending_approvals` audit row
                // and publishes `ApprovalRequested`. There is no chat thread to
                // route the prompt to for a background/triggered flow run yet
                // (B3 will add a dedicated review surface) — a caller can still
                // decide it via `approval_decide` (e.g. a generic pending-
                // approvals list) before the TTL elapses; absent a decision this
                // TTL-denies, the conservative fail-closed default for a
                // user-forced HITL gate.
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
                            "{POLICY_DENIED_MARKER} '{tool_name}' was blocked because this agent \
                             turn is missing its origin label, so the approval gate cannot decide \
                             who requested the action. Scheduling and other external-effect tools \
                             (e.g. cron_add / cron_update) are refused when the turn has no origin. \
                             This is an internal wiring gap, not something you did — the work most \
                             likely ran on a background task that did not carry the turn's origin \
                             forward; retry from a normal chat turn, or report it so the spawn site \
                             can be fixed."
                        ),
                    },
                    None,
                );
            }
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        // Resolve the clamped park TTL up front so the persisted `expires_at`
        // and the actual wait below (see `resolve_park_ttl` further down)
        // use the same value — see `Self::resolve_park_ttl` and the
        // COPILOT_APPROVAL_TTL clamp. Computing this
        // only after persisting the pending row let a copilot-streaming park
        // advertise the old 10-minute `expires_at` while only actually
        // waiting 180s, so a core restart or an `expire_stale` sweep mid-park
        // could leave the row "actionable" for the wrong window (CodeRabbit
        // + Codex review on PR #5112).
        let effective_ttl = Self::resolve_park_ttl(self.effective_ttl(), copilot_stream);
        let expires_at = Some(now + chrono::Duration::from_std(effective_ttl).unwrap_or_default());

        // Correlation context (flow-approval-surface, PR2): a Workflow-origin
        // park carries the flow id on the origin itself, but not the run id —
        // that comes from the `APPROVAL_FLOW_RUN_CONTEXT` task-local
        // `flows::ops::flows_run`/`flows_resume` scope alongside `with_origin`.
        // `try_with` returns `Err` for every non-flow caller (chat, cron,
        // subconscious, CLI, and even a Workflow origin reached without the
        // flows module's scope, which "should never happen" but must not
        // panic), so `source_context` stays `None` there — unchanged chat
        // behavior.
        let source_context = match &origin {
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Workflow { .. },
                job_id: flow_id,
            } => APPROVAL_FLOW_RUN_CONTEXT
                .try_with(|ctx| ApprovalSourceContext::Flow {
                    flow_id: flow_id.clone(),
                    run_id: ctx.run_id.clone(),
                    node_id: None,
                })
                .ok(),
            _ => None,
        };

        let pending = PendingApproval {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted: args_redacted.clone(),
            created_at: now,
            expires_at,
            source_context: source_context.clone(),
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
            self.clear_thread(&chat_thread_id, &request_id);
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
        BUS.publish(DomainEvent::ApprovalRequested {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            action_summary: action_summary.to_string(),
            args_redacted,
            thread_id: chat_thread_id.clone(),
            client_id: chat_client_id.clone(),
        });

        // Flow-origin surface bridge (flow-approval-surface, PR3): a flow run
        // has no chat thread/client to route the generic `ApprovalRequested`
        // through (both are `None` above, so the web-channel bridge silently
        // drops it — see `web_chat::event_bus`'s
        // `ApprovalSurfaceSubscriber`), which is exactly the silent-deadlock
        // bug this correlation fixes. Broadcast a dedicated
        // `flow_approval_request` socket event (no thread/client required,
        // unlike the chat path) plus a `CoreNotification` with the three
        // flow-scoped decision actions, so the Workflows UI can surface and
        // resolve the park without polling.
        if let Some(ApprovalSourceContext::Flow {
            flow_id, run_id, ..
        }) = &source_context
        {
            tracing::info!(
                request_id = %request_id,
                flow_id = %flow_id,
                run_id = %run_id,
                tool = tool_name,
                "[approval::gate] flow-origin park — surfacing flow_approval_request + notification"
            );
            BUS.publish(DomainEvent::FlowApprovalRequested {
                request_id: request_id.clone(),
                flow_id: flow_id.clone(),
                run_id: run_id.clone(),
                tool_name: tool_name.to_string(),
                summary: action_summary.to_string(),
            });
            // The workspace the flow parked in, so the approval banner is
            // dropped by a client that has since switched away rather than
            // approving this workspace's call from another one. Fails open on
            // a resolve failure: an unbound notification still reaches the
            // user, whereas not publishing recreates the silent deadlock this
            // bridge exists to fix.
            let workspace = match crate::openhuman::config::active_workspace_snapshot().await {
                Ok((dir, revision)) => {
                    Some((crate::openhuman::config::workspace_handle(&dir), revision))
                }
                Err(error) => {
                    tracing::warn!(
                        request_id = %request_id,
                        "[approval::gate] could not resolve the active workspace for the flow approval notification ({error}); publishing it unbound"
                    );
                    None
                }
            };
            publish_flow_gate_notification(
                &request_id,
                flow_id,
                run_id,
                tool_name,
                action_summary,
                workspace,
            );
        }

        tracing::info!(
            request_id = %request_id,
            tool = tool_name,
            "[approval::gate] tool call parked, waiting for decision"
        );

        // Copilot-streaming flows_build runs get a clamped park window — see
        // COPILOT_APPROVAL_TTL and `Self::resolve_park_ttl`. `effective_ttl` was resolved above
        // (before `expires_at` was built) so the persisted expiry and this
        // wait use the identical clamped duration; `effective_ttl()` applies
        // the debug-only env override, and the clamp is applied on top so a
        // longer override can't extend either park past its clamp.
        if copilot_stream {
            tracing::debug!(
                tool = tool_name,
                ttl_secs = COPILOT_APPROVAL_TTL.as_secs(),
                "[approval::gate] flows_build copilot-streaming park — clamping park window to \
                 COPILOT_APPROVAL_TTL"
            );
        }

        // Optional caller-supplied park bound (issue #4756). A caller
        // (`composio_connect`) can cap how long the gate parks so a turn
        // degrades to a fast prompt instead of blocking to the full TTL.
        // Bounding must never *extend* the park, so we wait `min(bound, ttl)`;
        // the caller-bound abandon path fires only when the bound is what
        // elapses (`park_bound_active`).
        let park_bound_active = matches!(park_bound, Some(b) if b < effective_ttl);
        let wait = match park_bound {
            Some(b) => b.min(effective_ttl),
            None => effective_ttl,
        };

        // RAII cleanup for external teardown (#4774): if the turn future is
        // dropped while parked on the await below (the #4746/#4751 wall-clock
        // backstop firing), the match arms never run, so this guard evicts the
        // waiter, clears routing, and denies the pending row on drop. Disarmed
        // right after the match on every normal exit.
        let mut waiter_guard = WaiterGuard {
            gate: self,
            request_id: request_id.clone(),
            thread_id: chat_thread_id.clone(),
            armed: true,
        };

        let outcome = match tokio::time::timeout(wait, rx).await {
            Ok(Ok(decision)) => {
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    decision = decision.as_str(),
                    "[approval::gate] decision received"
                );
                if decision.is_approve() {
                    (GateOutcome::Allow, Some(request_id.clone()))
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
            Err(_elapsed) if park_bound_active => {
                // Caller park bound elapsed (#4756) — NOT the gate's own TTL.
                // Abandon the park cancellation-safely: evict the in-memory
                // waiter and (via `clear_thread` below, on every
                // exit) drop the routing mappings so a later chat/voice reply is
                // not mis-routed to this now-abandoned request. Deliberately do
                // NOT `store::decide(Deny)` — the `pending_approvals` row stays
                // open so a later human card-click still resolves it in the DB
                // and a re-ask sees it already-connected. Signal the elapse so
                // the bounded caller renders its own fast-path result rather than
                // a `Deny`.
                self.evict_waiter(&request_id);
                *park_bound_elapsed = true;
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    bound_secs = wait.as_secs(),
                    "[approval::gate] caller park bound elapsed — abandoning park (row left \
                     pending for a later card-click; waiter + routing cleared) (#4756)"
                );
                // Placeholder outcome; the bounded caller discards it once
                // `*park_bound_elapsed` is set (returns `None`).
                (
                    GateOutcome::Deny {
                        reason: format!(
                            "{POLICY_DENIED_MARKER} Approval for '{tool_name}' exceeded the caller \
                             park bound ({}s).",
                            wait.as_secs()
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
                    (GateOutcome::Allow, Some(request_id.clone()))
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
        // Reached only on a normal park resolution: the match arm above already
        // ran the exact teardown for its outcome, so disarm the RAII guard (its
        // Drop is reserved for external cancellation — see `WaiterGuard`).
        waiter_guard.disarm();
        // The routing mappings are only needed while parked; clear them on
        // every exit (decision, channel drop, or timeout).
        self.clear_thread(&chat_thread_id, &request_id);
        outcome
    }
}
