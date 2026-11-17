async fn find_reusable_driver_session(
    driver: &std::sync::Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    selector: &SubagentSessionSelector,
) -> Result<Option<DurableSubagentSession>, String> {
    let driver_selector = tinyagents_orchestration::subagent::SubagentSessionSelector {
        parent_session: selector.parent_session.clone(),
        parent_thread_id: selector.parent_thread_id.clone(),
        agent_id: selector.agent_id.clone(),
        model: selector.model.clone(),
        sandbox_mode: selector.sandbox_mode.clone(),
        action_root: selector.action_root.clone(),
        task_key: selector.task_key.clone(),
    };
    let sessions = driver
        .list_for_parent(
            &selector.parent_session,
            selector.parent_thread_id.as_deref(),
        )
        .await
        .map_err(|error| error.to_string())?;
    let Some(selected) =
        tinyagents_orchestration::subagent::find_reusable_session(sessions, &driver_selector)
    else {
        return Ok(None);
    };
    let parent_epoch = subagent_sessions::parent_epoch_for_context(
        ctx,
        workspace_dir,
        selected.subagent_session_id.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(Some(subagent_sessions::host_projection(
        selected,
        parent_epoch,
    )))
}

async fn reusable_driver_session_for_spawn(
    driver: &std::sync::Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    host_store: &SubagentSessionStore,
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    selector: &SubagentSessionSelector,
    force_fresh: bool,
) -> Result<Option<DurableSubagentSession>, String> {
    let lookup = find_reusable_driver_session(driver, ctx, workspace_dir, selector).await;
    if force_fresh {
        match lookup {
            Ok(Some(session)) => {
                close_reusable_driver_session(
                    driver,
                    host_store,
                    ctx,
                    workspace_dir,
                    &session,
                    &selector.parent_session,
                    selector.parent_thread_id.as_deref(),
                )
                .await
                ?;
            }
            Ok(None) => {}
            Err(error) => log::warn!(
                "[subagent_reuse] fresh lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                selector.parent_thread_id.as_deref().unwrap_or("none"),
                selector.agent_id,
                selector.task_key,
                error
            ),
        }
        Ok(None)
    } else {
        match lookup {
            Ok(session) => Ok(session),
            Err(error) => {
                log::warn!(
                    "[subagent_reuse] lookup failed parent_thread_id={} agent_id={} task_key={} error={}",
                    selector.parent_thread_id.as_deref().unwrap_or("none"),
                    selector.agent_id,
                    selector.task_key,
                    error
                );
                Ok(None)
            }
        }
    }
}

async fn upsert_driver_session(
    driver: &std::sync::Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    upsert: SubagentSessionUpsert,
    force_fresh: bool,
) -> Result<DurableSubagentSession, String> {
    let lifecycle_key = upsert
        .lifecycle_key
        .ok_or_else(|| "driver session upsert requires a lifecycle key".to_owned())?;
    let parent_epoch = upsert.parent_epoch;
    let tiny_selector = tinyagents_orchestration::subagent::SubagentSessionSelector {
        parent_session: upsert.selector.parent_session,
        parent_thread_id: upsert.selector.parent_thread_id,
        agent_id: upsert.selector.agent_id,
        model: upsert.selector.model,
        sandbox_mode: upsert.selector.sandbox_mode,
        action_root: upsert.selector.action_root,
        task_key: upsert.selector.task_key,
    };
    let (session, _) = driver
        .upsert_running(tinyagents_orchestration::subagent::SubagentSessionUpsert {
            selector: tiny_selector,
            lifecycle_key,
            display_name: upsert.display_name,
            task_title: upsert.task_title,
            worker_thread_id: upsert.worker_thread_id,
            task_id: upsert.task_id,
            force_fresh,
        })
        .await
        .map_err(|error| error.to_string())?;
    crate::agent::orchestration::subagent_sessions::set_parent_epoch_for_context(
        ctx,
        workspace_dir,
        session.subagent_session_id.clone(),
        parent_epoch,
    )
    .map_err(|error| error.to_string())?;
    Ok(crate::agent::orchestration::subagent_sessions::host_projection(session, parent_epoch))
}

async fn close_reusable_driver_session(
    driver: &std::sync::Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    host_store: &SubagentSessionStore,
    ctx: &DispatchCtx,
    workspace_dir: &std::path::Path,
    session: &DurableSubagentSession,
    parent_session: &str,
    parent_thread_id: Option<&str>,
) -> Result<(), String> {
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(ctx);
    let cancelled = subagent_sessions::cancel_driver_job_for_observed_session(
        host_store,
        &workspace_dir.join(".openhuman/subagent_checkpoints"),
        &jobs,
        session,
        parent_session,
        parent_thread_id,
    );
    if !cancelled {
        let _ = running_subagents::cancel_by_session_in_workspace(
            ctx,
            &session.subagent_session_id,
            parent_session,
            workspace_dir,
        )
        .await;
    }
    let (Some(task_id), Some(lifecycle_key)) = (
        session.current_task_id.as_deref(),
        session.lifecycle_key.as_ref(),
    ) else {
        return Err("reusable session has no current lifecycle identity".into());
    };
    driver
        .close_if_current(
            &session.subagent_session_id,
            task_id,
            lifecycle_key,
            parent_session,
            parent_thread_id,
        )
        .await
        .map_err(|error| format!("failed to close reusable session: {error}"))?
        .then_some(())
        .ok_or_else(|| {
            "reusable session changed during fresh start; retry with the current session".into()
        })
}
