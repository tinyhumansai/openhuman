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

/// A delivered workflow proposal in the parent thread: the proposal message
/// id, the proposal name, the `draft_id` / `flow_id` the proposal payload
/// carries when it derives from an existing draft or flow, and the flow id
/// the card (or the agent) recorded on it (`saved_flow_id`, #8450) when it
/// was saved. Listings are newest first; a later revision does not inherit
/// an earlier save, so only messages with their own `saved_flow_id` count
/// as saved.
pub(crate) struct ThreadWorkflowProposal {
    pub(crate) message_id: String,
    pub(crate) name: String,
    pub(crate) draft_id: Option<String>,
    pub(crate) flow_id: Option<String>,
    pub(crate) saved_flow_id: Option<String>,
}

impl ThreadWorkflowProposal {
    pub(crate) fn is_saved(&self) -> bool {
        self.saved_flow_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    }
}

/// The workflow proposals in the parent thread, newest first. Best-effort:
/// a store read failure returns an empty list and logs, like
/// `record_agent_saved_workflow`, so a lookup failure never blocks a spawn.
pub(crate) fn workflow_proposals_in_thread(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Vec<ThreadWorkflowProposal> {
    let messages = match conversations::get_messages(ctx, workspace_dir.to_path_buf(), thread_id) {
        Ok(messages) => messages,
        Err(error) => {
            log::warn!(
                "[spawn_subagent] saved-proposal lookup failed thread_id={thread_id} error={error}"
            );
            return Vec::new();
        }
    };
    messages
        .iter()
        .rev()
        .filter_map(|message| {
            let meta = &message.extra_metadata;
            if meta["scope"] != "workflow_proposal" {
                return None;
            }
            let proposal = &meta["proposal"];
            let name = proposal["name"].as_str()?;
            if name.is_empty() {
                return None;
            }
            let text_field = |field: &str| {
                proposal
                    .get(field)
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            Some(ThreadWorkflowProposal {
                message_id: message.id.clone(),
                name: name.to_string(),
                draft_id: text_field("draft_id"),
                flow_id: text_field("flow_id"),
                saved_flow_id: meta
                    .get("saved_flow_id")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            })
        })
        .collect()
}

/// The workflow proposals the user already saved from their cards, newest
/// first (#8450). See [`workflow_proposals_in_thread`].
pub(crate) fn already_saved_workflow_proposals(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Vec<ThreadWorkflowProposal> {
    workflow_proposals_in_thread(ctx, workspace_dir, thread_id)
        .into_iter()
        .filter(ThreadWorkflowProposal::is_saved)
        .collect()
}

/// Whether `token` occurs in `text` as a delimited token, not as part of a
/// longer id: the characters on either side must not be id characters
/// (`[A-Za-z0-9_-]`). Ids (`draft_id` / `flow_id`) are matched this way so a
/// short id never matches a longer one that merely contains it.
fn contains_token(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let is_id_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    text.match_indices(token).any(|(at, _)| {
        let before_ok = at == 0 || !text[..at].chars().last().is_some_and(is_id_char);
        let after_at = at + token.len();
        let after_ok =
            after_at == text.len() || !text[after_at..].chars().next().is_some_and(is_id_char);
        before_ok && after_ok
    })
}

/// The proposal a `workflow_builder` hand-off is about, if the hand-off
/// identifies one. Identity first: a `draft_id` or `flow_id` the hand-off
/// carries, as the orchestrator is told to pass them. Then by exact name.
/// Names are not unique, so a name match takes the longest proposal name the
/// hand-off carries — a hand-off naming "Daily Report" must not match a saved
/// "Daily" — and among same-length names the newest proposal wins: when the
/// newest proposal with that name is still unsaved, the hand-off is about the
/// newer one, not an older saved namesake.
fn hand_off_proposal<'a>(
    proposals: &'a [ThreadWorkflowProposal],
    hand_off: &str,
) -> Option<&'a ThreadWorkflowProposal> {
    for proposal in proposals {
        let identified = [proposal.draft_id.as_deref(), proposal.flow_id.as_deref()]
            .into_iter()
            .flatten()
            .any(|id| contains_token(hand_off, id));
        if identified {
            return Some(proposal);
        }
    }
    let mut best: Option<&ThreadWorkflowProposal> = None;
    for proposal in proposals {
        if !hand_off.contains(proposal.name.as_str()) {
            continue;
        }
        let longer = match best {
            None => true,
            Some(current) => proposal.name.len() > current.name.len(),
        };
        if longer {
            best = Some(proposal);
        }
    }
    best
}

/// Note appended to a `workflow_builder` save hand-off when the proposal it
/// names was already saved from its card, so the builder reports it as
/// already saved instead of calling `create_workflow` and duplicating the
/// flow (#8459). The `save_workflow` escape hatch covers a graph that changed
/// alongside the save. Scoped to saving this proposal as-is: an explicit
/// request for a separate variant or copy still creates a new flow.
fn already_saved_proposal_note(name: &str, flow_id: &str) -> String {
    format!(
        "Note: the workflow proposal \"{name}\" was already saved from its card as flow `{flow_id}`. \
         This applies only to saving this proposal as-is. Report it as already saved as `{flow_id}`. \
         Do not call `create_workflow` for it. \
         If its graph changed and needs updating, use `save_workflow` with flow_id `{flow_id}` \
         instead of creating a new flow. \
         If the request instead asks for a separate variant or copy of it, create that as a new flow as requested."
    )
}

/// Carry a card-saved proposal into a `workflow_builder` hand-off (#8459):
/// when the parent thread holds a proposal already saved from its card
/// (`saved_flow_id`, #8450) and the hand-off — prompt or context — identifies
/// that proposal (see [`hand_off_proposal`]), append a note so the builder
/// reports it as already saved instead of calling `create_workflow` and
/// creating a second copy. The carry decision is logged with the thread and
/// flow ids, never the prompt or the proposal name. Returns the prompt
/// unchanged when there is no parent thread, no proposal is identified, or
/// the identified proposal is still unsaved.
pub(crate) fn carry_saved_flow(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &Option<String>,
    prompt: String,
    context: Option<&str>,
) -> String {
    let Some(thread_id) = thread_id.as_deref() else {
        return prompt;
    };
    let proposals = workflow_proposals_in_thread(ctx, workspace_dir, thread_id);
    let hand_off = match context {
        Some(context) if !context.trim().is_empty() => format!("{prompt}\n{context}"),
        _ => prompt.clone(),
    };
    let Some(proposal) = hand_off_proposal(&proposals, &hand_off) else {
        log::debug!(
            "[spawn_subagent] saved-flow carry skipped thread_id={thread_id} reason=no-matching-proposal"
        );
        return prompt;
    };
    let Some(flow_id) = proposal
        .saved_flow_id
        .as_deref()
        .filter(|id| !id.is_empty())
    else {
        log::debug!(
            "[spawn_subagent] saved-flow carry skipped thread_id={thread_id} proposal_id={} reason=proposal-unsaved",
            proposal.message_id
        );
        return prompt;
    };
    log::debug!(
        "[spawn_subagent] saved-flow carry applied thread_id={thread_id} proposal_id={} flow_id={flow_id}",
        proposal.message_id
    );
    format!(
        "{prompt}\n\n{}",
        already_saved_proposal_note(&proposal.name, flow_id)
    )
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
