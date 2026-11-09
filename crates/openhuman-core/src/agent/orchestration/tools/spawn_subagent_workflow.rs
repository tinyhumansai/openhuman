// Settling a finished child's workflow output in the parent thread: surfacing
// the proposal a `workflow_builder` run produced, and recording a flow it
// persisted onto the proposal the user is looking at.
//
// Textually included into `spawn_subagent_surface.rs`, so it shares that
// module's imports. Split out to keep the surface under the layout check's
// line limit.

/// Scan a finished child's history for the LAST `workflow_proposal` tool
/// result (the workflow_builder's `propose_workflow` / `revise_workflow` /
/// `edit_workflow` all return `{"type":"workflow_proposal", ...}` JSON).
/// Returns the parsed payload, or `None` when the run produced no proposal.
/// Lives here (not in `flows`) so the always-on orchestration path has no
/// dependency on the feature-gated flows domain — it is a generic scan for a
/// structured tool payload.
pub(crate) fn extract_workflow_proposal_from_history(
    history: &[TranscriptMessage],
) -> Option<serde_json::Value> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "tool")
        .find_map(|message| {
            let value: serde_json::Value = serde_json::from_str(message.content.trim()).ok()?;
            (value.get("type").and_then(|t| t.as_str()) == Some("workflow_proposal"))
                .then_some(value)
        })
}

/// Settle a finished child's workflow output in the parent thread: surface a
/// proposal it produced (see [`persist_workflow_proposal`]), then record a flow
/// it persisted onto the thread's live proposal (see
/// [`record_agent_saved_workflow`]), in that order, so a save made in the same
/// run settles the proposal the user is now looking at.
fn attach_workflow_proposal(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    parent_thread_id: Option<&str>,
    task_id: &str,
    agent_id: &str,
    final_history: &[TranscriptMessage],
    summary: String,
) -> String {
    let summary = persist_workflow_proposal(
        ctx,
        workspace_dir,
        parent_thread_id,
        task_id,
        agent_id,
        final_history,
        summary,
    );
    if let Some(thread_id) = parent_thread_id {
        record_agent_saved_workflow(ctx, workspace_dir, thread_id, task_id, final_history);
    }
    summary
}

/// The history a reused session's run starts from (its stored history plus the
/// follow-up prompt) and that history's length, after which the run's own
/// messages begin (see [`this_run_history`]). `(None, 0)` for a fresh session.
pub(crate) fn reused_session_history(
    reusable: Option<&DurableSubagentSession>,
    follow_up_prompt: &str,
) -> (Option<Vec<TranscriptMessage>>, usize) {
    let history = reusable
        .and_then(|session| session.latest_history.clone())
        .map(|mut history| {
            history.push(TranscriptMessage::user(follow_up_prompt.to_owned()));
            history
        });
    let seed_len = history.as_ref().map_or(0, Vec::len);
    (history, seed_len)
}

/// The part of a finished child's history this run produced. A reused session
/// starts from its stored history (`seed_len` messages, the follow-up prompt
/// included) and returns it whole, so an earlier turn's proposal or save would
/// otherwise be found again: re-persisted as a duplicate proposal message, or
/// recorded against a later proposal. A history shorter than its seed (it was
/// compacted) cannot be split and is used whole, as before.
pub(crate) fn this_run_history(
    history: &[TranscriptMessage],
    seed_len: usize,
) -> &[TranscriptMessage] {
    match history.get(seed_len..) {
        Some(run) => run,
        None => {
            log::debug!(
                "[spawn_subagent] history ({} messages) shorter than its seed ({seed_len}); using it whole",
                history.len()
            );
            history
        }
    }
}

/// The flow a finished child persisted from a proposal: the LAST
/// `workflow_created` (`create_workflow`) or `workflow_saved` (`save_workflow`)
/// tool result, as `(flow_id, name)`. `None` when the run produced a proposal
/// after that save: the save then belongs to an earlier graph, not to the
/// revision now in front of the user.
pub(crate) fn extract_saved_workflow_from_history(
    history: &[TranscriptMessage],
) -> Option<(String, String)> {
    let tool_payload = |message: &TranscriptMessage| -> Option<serde_json::Value> {
        (message.role == "tool")
            .then(|| serde_json::from_str(message.content.trim()).ok())
            .flatten()
    };
    let kind_of = |value: &serde_json::Value| -> Option<String> {
        value
            .get("type")
            .and_then(|t| t.as_str())
            .map(str::to_string)
    };
    let (save_at, saved) = history.iter().enumerate().rev().find_map(|(at, message)| {
        let value = tool_payload(message)?;
        let kind = kind_of(&value)?;
        (kind == "workflow_created" || kind == "workflow_saved").then_some((at, value))
    })?;
    let proposed_later = history[save_at + 1..].iter().any(|message| {
        tool_payload(message)
            .and_then(|value| kind_of(&value))
            .is_some_and(|kind| kind == "workflow_proposal")
    });
    if proposed_later {
        return None;
    }
    let flow_id = saved.get("flow_id").and_then(|v| v.as_str())?;
    let name = saved.get("name").and_then(|v| v.as_str()).unwrap_or("");
    Some((flow_id.to_string(), name.to_string()))
}

/// A flow the agent persisted from a delivered proposal (the user said "save
/// it" in chat rather than clicking the card) is recorded on the thread's live
/// proposal message as `saved_flow_id`, the same field the card writes. The
/// card then resumes at the enable step for that flow instead of creating a
/// second copy. Only the newest proposal message counts, and only while it is
/// unsettled and has the saved flow's name, so an unrelated proposal is never
/// claimed. Best-effort: a failure is logged and the delivery is unaffected.
fn record_agent_saved_workflow(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
    task_id: &str,
    final_history: &[TranscriptMessage],
) {
    let Some((flow_id, name)) = extract_saved_workflow_from_history(final_history) else {
        return;
    };
    let messages = match conversations::get_messages(ctx, workspace_dir.to_path_buf(), thread_id) {
        Ok(messages) => messages,
        Err(err) => {
            log::warn!(
                "[spawn_subagent] agent-saved workflow: could not read thread_id={thread_id} \
                 task_id={task_id} error={err}"
            );
            return;
        }
    };
    let Some(proposal) = messages
        .iter()
        .rev()
        .find(|m| m.extra_metadata["scope"] == "workflow_proposal")
    else {
        log::debug!(
            "[spawn_subagent] agent-saved workflow: no proposal in thread_id={thread_id} \
             task_id={task_id} flow_id={flow_id}"
        );
        return;
    };
    let meta = &proposal.extra_metadata;
    let settled = meta["consumed"] == true || meta.get("saved_flow_id").is_some();
    if settled || meta["proposal"]["name"] != name.as_str() {
        log::debug!(
            "[spawn_subagent] agent-saved workflow: newest proposal {} not claimed \
             (settled={settled}) task_id={task_id} flow_id={flow_id}",
            proposal.id
        );
        return;
    }
    let patch = conversations::ConversationMessagePatch {
        merge_extra_metadata: Some(json!({ "saved_flow_id": flow_id })),
        ..Default::default()
    };
    match conversations::update_message(
        ctx,
        workspace_dir.to_path_buf(),
        thread_id,
        &proposal.id,
        patch,
    ) {
        Ok(_) => log::info!(
            "[spawn_subagent] agent-saved workflow recorded on proposal {} \
             flow_id={flow_id} task_id={task_id}",
            proposal.id
        ),
        Err(err) => log::warn!(
            "[spawn_subagent] agent-saved workflow: could not mark proposal {} \
             flow_id={flow_id} task_id={task_id} error={err}",
            proposal.id
        ),
    }
}

/// Durably surface a workflow proposal found in a finished child's history:
/// persist it as a parent-thread conversation message (metadata carries the
/// full payload so the UI can rehydrate the proposal card after reload) and
/// append a `[workflow_proposal]` envelope to the delivery summary so the
/// follow-up turn presents it faithfully. Returns the (possibly extended)
/// summary; on any persistence error the summary still carries the envelope —
/// losing durability must not lose delivery.
fn persist_workflow_proposal(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    parent_thread_id: Option<&str>,
    task_id: &str,
    agent_id: &str,
    final_history: &[TranscriptMessage],
    summary: String,
) -> String {
    let Some(proposal) = extract_workflow_proposal_from_history(final_history) else {
        return summary;
    };
    let proposal_json = match serde_json::to_string(&proposal) {
        Ok(json) => json,
        Err(err) => {
            log::warn!(
                "[spawn_subagent] workflow proposal re-serialize failed task_id={task_id} error={err}"
            );
            return summary;
        }
    };
    let name = proposal
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("Untitled workflow");
    log::info!(
        "[spawn_subagent] extracted workflow proposal '{name}' task_id={task_id} \
         ({} chars) — persisting to parent thread {:?}",
        proposal_json.len(),
        parent_thread_id
    );
    if let Some(thread_id) = parent_thread_id {
        let persisted = conversations::append_message(
            ctx,
            workspace_dir.to_path_buf(),
            thread_id,
            ConversationMessage {
                id: format!("workflow-proposal:{task_id}"),
                content: format!("Workflow proposal ready: {name}"),
                message_type: "text".to_string(),
                extra_metadata: json!({
                    "scope": "workflow_proposal",
                    "proposal": proposal,
                    "task_id": task_id,
                    "agent_id": agent_id,
                }),
                sender: "agent".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        if let Err(err) = persisted {
            log::warn!(
                "[spawn_subagent] workflow proposal persistence failed \
                 thread_id={thread_id} task_id={task_id} error={err} — proposal still \
                 rides the delivery notice"
            );
        }
    }
    format!(
        "{summary}\n\n[workflow_proposal]\n{proposal_json}\n[/workflow_proposal]\n\
         (The full proposal above was also saved to the chat thread; present it to the \
         user for review — do not re-run the builder unless they ask for changes.)"
    )
}
