// Off-worker saved-flow checks for a live `workflow_builder` hand-off: the
// `flows.get` presence probe, and releasing a receipt for a flow that no
// longer exists so its recreation can be recorded.
//
// Textually included into `spawn_subagent_surface.rs`, so it shares that
// module's imports. Split out of `spawn_subagent_workflow.rs` to keep it under
// the layout check's line limit.

/// [`saved_flow_presence`] on the blocking pool. The `flows.get` handler
/// reaches the flow catalog through the storage bridge, which waits
/// synchronously (up to the catalog proxy's timeout); on the live spawn path
/// that must not park an async worker. The probe runs under `ctx`. A task
/// failure reads as `Unknown`, which keeps the note, like a lookup error.
async fn saved_flow_presence_off_worker(
    ctx: &DispatchCtx,
    thread_id: &str,
    flow_id: &str,
) -> SavedFlowPresence {
    let probe_ctx = ctx.clone();
    let scope_ctx = ctx.clone();
    let owned_flow_id = flow_id.to_string();
    let handle = tokio::runtime::Handle::current();
    let probe = tokio::task::spawn_blocking(move || {
        handle.block_on(scope_ctx.enter(async move {
            saved_flow_presence(&probe_ctx, &owned_flow_id).await
        }))
    })
    .await;
    match probe {
        Ok(presence) => presence,
        Err(error) => {
            log::warn!(
                "[spawn_subagent] saved-flow presence probe failed thread_id={thread_id} \
                 flow_id={flow_id} error={error}"
            );
            SavedFlowPresence::Unknown
        }
    }
}

/// Drop a proposal's receipt for a flow that no longer exists (its
/// `saved_flow_id`, and the `consumed` mark a card save sets), so the proposal
/// reads as unsaved again and [`record_agent_saved_workflow`] can record the
/// flow the builder recreates. Only a receipt still naming `flow_id` is
/// released, so a save that landed meanwhile is kept. Best-effort: failures
/// are logged.
fn release_missing_saved_flow(
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
    proposal_id: &str,
    flow_id: &str,
) {
    let still_names_it = workflow_proposals_in_thread(ctx, workspace_dir, thread_id)
        .iter()
        .any(|p| p.message_id == proposal_id && p.saved_flow_id.as_deref() == Some(flow_id));
    if !still_names_it {
        log::debug!(
            "[spawn_subagent] saved-flow receipt already changed thread_id={thread_id} \
             proposal_id={proposal_id} flow_id={flow_id}"
        );
        return;
    }
    let patch = conversations::ConversationMessagePatch {
        merge_extra_metadata: Some(json!({ "saved_flow_id": null, "consumed": null })),
        ..Default::default()
    };
    match conversations::update_message(
        ctx,
        workspace_dir.to_path_buf(),
        thread_id,
        proposal_id,
        patch,
    ) {
        Ok(_) => log::info!(
            "[spawn_subagent] released missing saved-flow receipt thread_id={thread_id} \
             proposal_id={proposal_id} flow_id={flow_id}"
        ),
        Err(error) => log::warn!(
            "[spawn_subagent] could not release saved-flow receipt thread_id={thread_id} \
             proposal_id={proposal_id} flow_id={flow_id} error={error}"
        ),
    }
}
