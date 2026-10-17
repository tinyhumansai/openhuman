// Async delivery is thread-addressed: the finished result is inserted back into the
// parent chat thread as a follow-up turn (`background_delivery`). Outside a chat turn
// (flow `agent` nodes, CLI and cron runs intentionally have no parent thread to deliver
// into, so `background_delivery::deliver_batch` logs "dropping headless batch" and the
// (possibly real, completed) work is silently discarded — the caller sees "Accepted" and
// never learns the result never arrived. Fail loudly instead: the caller has a
// synchronous alternative (`spawn_subagent` with `blocking: true`, or a `delegate_*`
// tool). Both of those self-heal to blocking dispatch in this situation rather than
// reaching this guard — see the `has_delivery_thread` checks in `spawn_subagent.rs` and
// `dispatch.rs::dispatch_subagent`. Only a *direct* `spawn_async_subagent` call lands
// here.
fn refuse_missing_delivery_thread(
    parent_thread_id: Option<&str>,
    parent_agent_id: &str,
    requested_agent_id: &str,
) -> Option<ToolResult> {
    if parent_thread_id.is_none() {
        log::warn!(
            "[spawn_async_subagent] refusing fire-and-forget spawn with no delivery thread \
                 parent={} requested={} — directing caller to synchronous delegation (flow node / \
                 CLI / cron context, background result would be discarded)",
            parent_agent_id,
            requested_agent_id
        );
        return Some(ToolResult::error(
            "spawn_async_subagent: no parent chat thread available to deliver the result \
                 into (this looks like a flow node, CLI, or cron run rather than an interactive \
                 chat turn). Fire-and-forget delegation has nowhere to land its result here and \
                 the sub-agent's work would be silently discarded. Use synchronous delegation \
                 instead: a `delegate_*` tool with `blocking: true` runs the sub-agent inline \
                 and hands you its output in this turn. For parallel work, model it as \
                 parallel flow nodes rather than background sub-agents.",
        ));
    }
    None
}

// The follow-up vocabulary offered back to the parent is limited to
// the fleet tools visible on *this turn* (see `fleet_tools`), not
// just the parent definition's static scope: a hide or named
// restriction can narrow the turn's real tool surface below what the
// definition alone would suggest, and offering a control the parent
// cannot currently call invites a denied tool call.
fn accepted_spawn_result(
    task_id: &str,
    session_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    fleet: &FleetToolSet,
) -> ToolResult {
    let payload = async_subagent_ref_payload(
        task_id,
        session_id,
        agent_id,
        worker_thread_id,
        reused,
        reuse_decision,
        "running",
        fleet,
    );
    let payload_json = match serde_json::to_string(&payload) {
        Ok(serialized) => {
            log::debug!(
                "[spawn_async_subagent] serialized async reference payload bytes={}",
                serialized.len()
            );
            serialized
        }
        Err(error) => {
            log::debug!(
                "[spawn_async_subagent] failed to serialize async reference payload: {}",
                error
            );
            "{}".to_string()
        }
    };
    log::debug!("[spawn_async_subagent] formatting accepted response");
    ToolResult::success(format_async_subagent_accepted(
        payload["agent_id"].as_str().unwrap_or("subagent"),
        &payload_json,
        fleet,
    ))
}

fn refuse_empty_spawn_args(agent_id: &str, prompt: &str) -> Option<ToolResult> {
    if agent_id.is_empty() {
        return Some(ToolResult::error(
            "spawn_async_subagent: `agent_id` is required",
        ));
    }
    if prompt.is_empty() {
        return Some(ToolResult::error(
            "spawn_async_subagent: `prompt` is required",
        ));
    }
    None
}
