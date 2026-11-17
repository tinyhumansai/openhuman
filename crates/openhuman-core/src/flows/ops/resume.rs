use super::*;
use tinyflows_bus::runs::ResumeRun;

/// Ask TinyFlows to continue a parked workflow after a human approval
/// decision. The module owns the persisted run snapshot, approval validation,
/// graph pin, compare-and-swap claim, checkpoint, actor, and terminal events.
/// OpenHuman binds the request to the caller's captured storage scope and
/// supplies a fresh capability callback that retains the same owner storage.
pub async fn flows_resume(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    thread_id: &str,
    approvals: Vec<String>,
    rejections: Vec<String>,
) -> Result<Outcome<Value>, String> {
    let workflow = current_workflow_for_resume(ctx, config, flow_id).await?;
    let bindings = crate::flows::runs_adapter::register_resume_bindings(
        ctx,
        config,
        flow_id,
        thread_id,
        workflow.require_approval,
    )
    .await?;
    let request = ResumeRun {
        run_id: thread_id.to_owned(),
        flow_id: flow_id.to_owned(),
        storage: bindings.storage(),
        caps_host: bindings.caps_host(),
        approvals,
        rejections,
        timeout_ms: FLOW_RUN_TIMEOUT_SECS.saturating_mul(1_000),
    };
    let resumed =
        crate::flows::runs_adapter::resume_and_wait(ctx, config, request, bindings).await?;

    tracing::info!(
        target: "flows",
        flow_id,
        run_id = %resumed.run_id,
        status = %resumed.status,
        pending_approvals = resumed.pending_approvals.len(),
        "[flows] flows_resume: module run segment settled"
    );
    Ok(Outcome::single_log(
        json!({
            "output": resumed.output,
            "pending_approvals": resumed.pending_approvals,
            "thread_id": resumed.run_id,
        }),
        format!("flow resume {}", resumed.status),
    ))
}

struct ResumeWorkflow {
    require_approval: bool,
}

/// Read the workflow through the same owner-scoped catalog callback that the
/// run adapter will use. This prevents a caller-supplied run or flow id from
/// resolving another account's approval policy for the fresh CapsHost.
/// Compatibility of the saved execution graph and its children is checked by
/// the owner-scoped Runs.ResumeRun operation.
async fn current_workflow_for_resume(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
) -> Result<ResumeWorkflow, String> {
    use tinyflows_bus::catalog::{members, Workflow};

    let storage = crate::flows::catalog_storage::register(ctx, config)
        .await?
        .storage();
    let proxy = crate::modules::tinyflows::catalog_proxy(config).await?;
    let workflow: Option<Workflow> = proxy
        .call(members::GET_WORKFLOW, (storage, flow_id.to_owned()))
        .await
        .map_err(|error| error.wire_message())?;
    let workflow = workflow.ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    Ok(ResumeWorkflow {
        require_approval: workflow.require_approval,
    })
}
