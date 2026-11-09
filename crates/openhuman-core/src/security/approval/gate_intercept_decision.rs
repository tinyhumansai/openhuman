impl ApprovalGate {
    /// Resolves the parked oneshot receiver against its wait bound
    /// (`min(park_bound, effective_ttl)`), handling every terminal shape: a
    /// decision arrives, the sender is dropped, the caller-supplied park
    /// bound elapses, or the request's own expiry elapses. Split out of
    /// [`Self::intercept_audited_inner`] (`gate_intercept.rs`) purely to keep
    /// that file under the repo's per-file line budget.
    ///
    /// Callers must still run the same post-match teardown
    /// (`guard.disarm()` + `self.parked.clear_thread(...)`) themselves; this
    /// helper only resolves the `outcome`.
    #[allow(clippy::too_many_arguments)]
    async fn resolve_park(
        &self,
        request_id: &str,
        tool_name: &str,
        wait: Duration,
        rx: oneshot::Receiver<ApprovalDecision>,
        park_bound_active: bool,
        park_bound_elapsed: &mut bool,
        effective_ttl: Duration,
        scope: Option<&super::ApprovalScope>,
        track: bool,
    ) -> (GateOutcome, Option<String>) {
        match tokio::time::timeout(wait, rx).await {
            Ok(Ok(decision)) => {
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    decision = decision.as_str(),
                    "[approval::gate] decision received"
                );
                if decision.is_approve() {
                    if tool_name == "native_secret_grant" {
                        // TinySecurity's ExecuteSecret2 consumes the native
                        // HumanOnce grant itself; a generic approval reservation
                        // would consume the row before the module can verify it.
                        self.engine.release(request_id);
                        (GateOutcome::Allow, None)
                    } else {
                        self.reserved(request_id.to_string(), tool_name, scope, track)
                            .await
                    }
                } else {
                    // Nothing will reserve or record a denied request.
                    self.engine.release(request_id);
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
                if let Err(error) = self.engine.cancel(request_id).await {
                    tracing::debug!(
                        request_id = %request_id,
                        error = %error,
                        "[approval::gate] the engine did not cancel the abandoned request"
                    );
                }
                self.publish_resolution(request_id, "cancelled");
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
                // Caller park bound elapsed (#4756) — NOT the request's own expiry.
                // Abandon the park cancellation-safely: evict the in-memory
                // waiter and (via `clear_thread` in the caller, on every
                // exit) drop the routing mappings so a later chat/voice reply is
                // not mis-routed to this now-abandoned request. Deliberately do
                // NOT cancel the request — it stays open so a later human
                // card-click still resolves it in the store and a re-ask sees it
                // already-connected. Signal the elapse so
                // the bounded caller renders its own fast-path result rather than
                // a `Deny`.
                self.parked.evict_waiter(request_id);
                *park_bound_elapsed = true;
                tracing::info!(
                    request_id = %request_id,
                    tool = tool_name,
                    bound_secs = wait.as_secs(),
                    "[approval::gate] caller park bound elapsed — abandoning park (request left \
                     open for a later card-click; waiter + routing cleared) (#4756)"
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
                self.parked.evict_waiter(request_id);
                // Race: a decision may have committed an Approve right as the
                // window elapsed. The engine expires only a request still
                // pending; when it was already decided, honor that decision
                // instead of denying a call the user approved.
                match self.expire_with_retry(request_id).await {
                    Ok(true) => {
                        tracing::warn!(
                            request_id = %request_id,
                            tool = tool_name,
                            ttl_secs = effective_ttl.as_secs(),
                            "[approval::gate] approval timed out, denying"
                        );
                        // Only this call expired the request, so only it
                        // announces the outcome: a concurrent decision already
                        // published its own.
                        self.publish_resolution(request_id, "expired");
                        (
                            GateOutcome::Deny {
                                reason: format!(
                                    "{POLICY_DENIED_MARKER} Approval for '{tool_name}' timed out after \
                                     {}s: {APPROVAL_UNANSWERED_PHRASE}, so it was not run. Tell the \
                                     user it was not done because the approval was not answered, and \
                                     that they can ask again to retry. Do not re-request the same call \
                                     this turn.",
                                    effective_ttl.as_secs()
                                ),
                            },
                            None,
                        )
                    }
                    Ok(false) => {
                        tracing::info!(
                            request_id = %request_id,
                            tool = tool_name,
                            ttl_secs = effective_ttl.as_secs(),
                            "[approval::gate] timeout race: the request was already decided, honoring the decision"
                        );
                        // Fall through (no early return) so `clear_thread` in the
                        // caller runs on this path too — otherwise the stale
                        // thread→request mapping survives and the next yes/no on
                        // the thread could be routed to this already-finished
                        // request. Reserving succeeds only for an approval; a caller
                        // that cannot report the outcome gets the safe answer.
                        if !track {
                            return (
                                GateOutcome::Deny {
                                    reason: format!(
                                        "{POLICY_DENIED_MARKER} Approval for '{tool_name}' was not granted in time."
                                    ),
                                },
                                None,
                            );
                        }
                        match self.engine.reserve(request_id).await {
                            Ok(()) => (GateOutcome::Allow, Some(request_id.to_string())),
                            Err(_) => (
                                GateOutcome::Deny {
                                    reason: format!(
                                        "{POLICY_DENIED_MARKER} Approval for '{tool_name}' was not granted."
                                    ),
                                },
                                None,
                            ),
                        }
                    }
                    Err(error) => {
                        tracing::error!(
                            request_id = %request_id,
                            tool = tool_name,
                            error = %error,
                            "[approval::gate] the request could not be expired — failing closed"
                        );
                        let _ = self.engine.cancel(request_id).await;
                        self.publish_resolution(request_id, "expired");
                        (
                            GateOutcome::Deny {
                                reason: format!(
                                    "{POLICY_DENIED_MARKER} Approval for '{tool_name}' timed out after \
                                     {}s: {APPROVAL_UNANSWERED_PHRASE}, so it was not run. Tell the \
                                     user it was not done because the approval was not answered, and \
                                     that they can ask again to retry. Do not re-request the same call \
                                     this turn.",
                                    effective_ttl.as_secs()
                                ),
                            },
                            None,
                        )
                    }
                }
            }
        }
    }

    /// Expire a request whose window elapsed. The engine refuses while its own
    /// clock still reads the request as live, which a wait that ended a
    /// millisecond early can hit, so a refusal is retried briefly.
    async fn expire_with_retry(&self, request_id: &str) -> Result<bool, EngineError> {
        let mut last = None;
        for attempt in 0..4u64 {
            match self.engine.expire(request_id).await {
                Ok(expired) => return Ok(expired),
                Err(EngineError::NotPending) => return Ok(false),
                Err(error) => last = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(50 * (attempt + 1))).await;
        }
        Err(last.unwrap_or(EngineError::NotPending))
    }

    /// Announce that a parked request ended without a person's decision
    /// (`resolution` is `cancelled` or `expired`), once, with the routing it was
    /// parked under.
    fn publish_resolution(&self, request_id: &str, resolution: &str) {
        let Some(route) = self.parked.take_request_route(request_id) else {
            return;
        };
        BUS.publish(DomainEvent::ApprovalDecided {
            request_id: request_id.to_string(),
            tool_name: route.pending.tool_name.clone(),
            decision: ApprovalDecision::Deny.as_str().to_string(),
            thread_id: route.thread_id.clone(),
            client_id: route.client_id.clone(),
            tool_call_id: route.tool_call_id.clone(),
            resolution: Some(resolution.to_string()),
            agent_id: route.agent_id.clone(),
        });
    }
}

/// Whether this SaaS call can ask the user through its authenticated WebChat
/// route. Calls without a routable thread/client remain subject to the closed
/// deployment allowlist.
pub(crate) fn saas_prompt_needed(
    ctx: &crate::core::runtime::DispatchCtx,
    saas: bool,
    origin: &AgentTurnOrigin,
    tool_name: &str,
    forced: bool,
) -> bool {
    let routable_web_chat = matches!(
        origin,
        AgentTurnOrigin::WebChat { thread_id, client_id, .. }
            if !thread_id.trim().is_empty() && !client_id.trim().is_empty()
    );
    saas
        && routable_web_chat
        && (forced || crate::profiles::tools::gate_verdict(ctx, tool_name).is_err())
}

/// Keep SaaS user approvals one-time even when the ordinary chat posture has
/// persistent grants enabled.
pub(crate) fn one_time_approval_mode(
    variant: Variant,
    forced: bool,
    saas_human_approval: bool,
) -> (Variant, bool) {
    if saas_human_approval {
        (Variant::Forced, true)
    } else {
        (variant, forced)
    }
}

/// The gate's outcome in SaaS, `None` when an authenticated per-user surface
/// should handle the approval (or outside SaaS). Without that route, calls use
/// the deployment allowlist and forced approvals fail closed.
pub(crate) fn saas_outcome_with(
    ctx: &crate::core::runtime::DispatchCtx,
    saas: bool,
    origin: &AgentTurnOrigin,
    forced: bool,
    tool_name: &str,
) -> Option<GateOutcome> {
    if !saas {
        return None;
    }
    if saas_prompt_needed(ctx, saas, origin, tool_name, forced) {
        return None;
    }
    if forced {
        return Some(GateOutcome::Deny {
            reason: format!(
                "{POLICY_DENIED_MARKER} A forced approval for '{tool_name}' needs a routable WebChat surface."
            ),
        });
    }
    Some(saas_outcome(crate::profiles::tools::gate_verdict(
        ctx, tool_name,
    )))
}

/// Maps a SaaS gate verdict to a [`GateOutcome`].
pub(crate) fn saas_outcome(verdict: Result<(), String>) -> GateOutcome {
    match verdict {
        Ok(()) => GateOutcome::Allow,
        Err(why) => GateOutcome::Deny {
            reason: format!("{POLICY_DENIED_MARKER} {why}"),
        },
    }
}
