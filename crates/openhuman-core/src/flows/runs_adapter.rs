//! OpenHuman adapters for TinyFlows' module-owned run lifecycle.
//!
//! The module owns execution, durable rows, checkpoints, cancellation, and
//! dedup settlement. This boundary retains scope-bound host callbacks,
//! projects ordered PollRun events into OpenHuman events, and preserves the
//! existing synchronous run result shape.

mod projection;
mod session;

use std::sync::Arc;

use serde_json::{json, Value};
use tinyflows_bus::catalog::Workflow as Flow;
use tinyflows_bus::runs::FlowRunTrigger;
use tinyflows_bus::runs::{self, ResumeRun, StartRun};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::flows::caps_host::{self, CapsHostRegistration};
use crate::flows::catalog_storage::{self, CatalogStorageRegistration};
use crate::modules::tinyflows as module;

pub(crate) const RUN_TIMEOUT_SECS: u64 = 600;
const RUN_TIMEOUT_MS: u64 = RUN_TIMEOUT_SECS * 1_000;

pub(crate) use session::{RunKey, RunSession};

/// Callback authorities retained for one owner-scoped flow run.
pub(crate) struct RunBindings {
    catalog: Arc<CatalogStorageRegistration>,
    caps: Arc<CapsHostRegistration>,
    pub(crate) flow: Flow,
    pub(crate) flow_id: String,
    pub(crate) run_id: String,
    pub(crate) require_approval: bool,
}

impl RunBindings {
    /// Stable owner scope used by every Catalog and Runs request.
    pub(crate) fn storage(&self) -> tinyflows_bus::catalog::CatalogStorage {
        self.catalog.storage()
    }

    /// Fresh callback bound to this run's flow, owner, and approval policy.
    pub(crate) fn caps_host(&self) -> runs::CapsHost {
        self.caps.address()
    }

    pub(crate) fn catalog_registration(&self) -> Arc<CatalogStorageRegistration> {
        Arc::clone(&self.catalog)
    }
}

/// A settled or parked engine segment, in the legacy controller vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunOutcome {
    pub(crate) run_id: String,
    pub(crate) output: Value,
    pub(crate) pending_approvals: Vec<String>,
    pub(crate) status: String,
}

/// Typed result returned to the existing `flows_resume` controller.
pub(crate) type ResumeOutcome = RunOutcome;

/// Return the retained bindings only for the exact owner scope and opaque run
/// id. This key is shared with the module's live Poll/Cancel owner identity.
pub(crate) fn run_bindings(
    storage: &tinyflows_bus::catalog::CatalogStorage,
    run_id: &str,
) -> Option<Arc<RunBindings>> {
    session::get(storage, run_id).map(|session| session.current_bindings())
}

/// Retain one run's owner callback and its per-run capability callback.
pub(crate) async fn register_bindings(
    ctx: &DispatchCtx,
    config: &Config,
    flow: &Flow,
    run_id: &str,
) -> Result<Arc<RunBindings>, String> {
    let catalog = catalog_storage::register(ctx, config).await?;
    register_with_catalog(ctx, config, flow, run_id, catalog).await
}

async fn register_with_catalog(
    ctx: &DispatchCtx,
    config: &Config,
    flow: &Flow,
    run_id: &str,
    catalog: Arc<CatalogStorageRegistration>,
) -> Result<Arc<RunBindings>, String> {
    let caps = caps_host::serve(
        ctx.clone(),
        Arc::new(config.clone()),
        flow.id.clone(),
        run_id.to_owned(),
        flow.require_approval,
        Arc::clone(&catalog),
    )
    .await?;
    Ok(Arc::new(RunBindings {
        catalog,
        caps: Arc::new(caps),
        flow: flow.clone(),
        flow_id: flow.id.clone(),
        run_id: run_id.to_owned(),
        require_approval: flow.require_approval,
    }))
}

/// Start a module-owned run and subscribe before its first PollRun projection.
pub(crate) async fn start_run(
    ctx: &DispatchCtx,
    config: &Config,
    flow: &Flow,
    run_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<
    (
        Arc<RunSession>,
        tokio::sync::mpsc::UnboundedReceiver<runs::RunEvent>,
    ),
    String,
> {
    let bindings = register_bindings(ctx, config, flow, run_id).await?;
    let session = RunSession::new(
        Arc::clone(&bindings),
        Arc::new(config.clone()),
        flow.clone(),
        trigger,
        0,
    );
    let receiver = session.subscribe();
    let graph = flow.graph.clone();
    let proxy = match module::runs_proxy(config).await {
        Ok(proxy) => proxy,
        Err(error) => return Err(error),
    };
    session::insert(Arc::clone(&session))?;
    let request = StartRun {
        run_id: run_id.to_owned(),
        context: runs::RunContext {
            flow_id: flow.id.clone(),
            origin: Some(json!({
                "kind": "workflow",
                "flow_id": flow.id.clone(),
                "require_approval": flow.require_approval,
            })),
        },
        caps_host: bindings.caps_host(),
        storage: bindings.storage(),
        graph,
        trigger_kind: bus_trigger(trigger),
        trigger_data: input,
        inputs,
        timeout_ms: RUN_TIMEOUT_MS,
    };
    let started: runs::StartRunResponse =
        match proxy.call(runs::methods::START_RUN, (request,)).await {
            Ok(started) => started,
            Err(error) => {
                match run_exists(config, bindings.storage(), run_id).await {
                    Ok(true) | Err(_) => session.start_polling(proxy)?,
                    Ok(false) => session::remove(&session),
                }
                return Err(error.wire_message());
            }
        };
    if started.run_id != run_id {
        // A malformed acknowledgement does not prove the module failed to
        // admit the requested actor. Keep both callbacks served until the
        // owner-scoped PollRun stream gives us a terminal answer.
        session.start_polling(proxy)?;
        return Err("TinyFlows admitted a different run id".to_string());
    }
    if started.already_running {
        session::remove(&session);
        return Err(format!("flow run '{run_id}' is already running"));
    }
    session.start_polling(proxy)?;
    Ok((session, receiver))
}

/// Wait for the first parked or terminal outcome after a run is admitted.
pub(crate) async fn wait_for_outcome(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<runs::RunEvent>,
    run_id: &str,
    after_seq: u64,
) -> Result<RunOutcome, String> {
    loop {
        let event = receiver
            .recv()
            .await
            .ok_or_else(|| "flow run event stream closed before settlement".to_string())?;
        if event.seq <= after_seq {
            continue;
        }
        match event.kind.as_str() {
            "parked" => {
                let parked: runs::RunParked = serde_json::from_value(event.data)
                    .map_err(|error| format!("decode parked run outcome: {error}"))?;
                return Ok(RunOutcome {
                    run_id: run_id.to_owned(),
                    output: parked
                        .output
                        .ok_or_else(|| "TinyFlows omitted the parked engine outcome".to_string())?,
                    pending_approvals: parked.pending_approvals,
                    status: "pending_approval".into(),
                });
            }
            "finished" => {
                let finished: runs::RunFinished = serde_json::from_value(event.data)
                    .map_err(|error| format!("decode finished run outcome: {error}"))?;
                return finished_outcome(finished, run_id);
            }
            _ => {}
        }
    }
}

/// Bind a fresh resume capability to the current owner and verify the parked
/// run is visible in that same CatalogStorage scope.
pub(crate) async fn register_resume_bindings(
    ctx: &DispatchCtx,
    config: &Config,
    flow_id: &str,
    run_id: &str,
    require_approval: bool,
) -> Result<Arc<RunBindings>, String> {
    use tinyflows_bus::catalog::{self, FlowRun as WireRun, Workflow};

    let catalog = catalog_storage::register(ctx, config).await?;
    let storage = catalog.storage();
    let runs = module::catalog_proxy(config).await?;
    let run: Option<WireRun> = runs
        .call(
            catalog::members::GET_RUN,
            (catalog::RunIdRequest {
                storage: storage.clone(),
                id: run_id.to_owned(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    let run = run.ok_or_else(|| format!("flow run '{run_id}' not found"))?;
    if run.flow_id != flow_id {
        return Err(format!(
            "flow run '{run_id}' does not belong to flow '{flow_id}'"
        ));
    }
    if run.status != "pending_approval" {
        return Err(format!("flow run '{run_id}' is not waiting for approval"));
    }

    let workflow: Option<Workflow> = runs
        .call(
            catalog::members::GET_WORKFLOW,
            (storage, flow_id.to_owned()),
        )
        .await
        .map_err(|error| error.wire_message())?;
    let flow: Flow = workflow.ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    if flow.require_approval != require_approval {
        return Err("workflow approval policy changed during resume".to_string());
    }

    if let Some(session) = session::get(&catalog.storage(), run_id) {
        if session.flow_id() != flow_id {
            return Err(format!(
                "flow run '{run_id}' is bound to a different workflow"
            ));
        }
    }
    register_with_catalog(ctx, config, &flow, run_id, catalog).await
}

/// Resume a parked module run and await only the next segment's parked or
/// terminal event. The prior parked cursor is drained before admission so a
/// process-restarted waiter cannot mistake an earlier `parked` event for the
/// result of this resume.
pub(crate) async fn resume_and_wait(
    _ctx: &DispatchCtx,
    config: &Config,
    request: ResumeRun,
    candidate: Arc<RunBindings>,
) -> Result<ResumeOutcome, String> {
    let storage = request.storage.clone();
    if candidate.flow_id != request.flow_id
        || candidate.run_id != request.run_id
        || candidate.storage() != storage
        || candidate.caps_host() != request.caps_host
    {
        return Err("resume bindings do not match the owner-scoped request".to_string());
    }
    let proxy = module::runs_proxy(config).await?;
    let (session, inserted) = match session::get(&storage, &request.run_id) {
        Some(session) => (session, false),
        None => {
            let session = RunSession::new(
                Arc::clone(&candidate),
                Arc::new(config.clone()),
                candidate.flow.clone(),
                FlowRunTrigger::Resume,
                0,
            );
            if session.drain(&proxy, false).await? {
                return Err(format!("flow run '{}' is already finished", request.run_id));
            }
            session::insert(Arc::clone(&session))?;
            (session, true)
        }
    };

    if !inserted {
        session.drain(&proxy, true).await?;
    }
    let baseline = session.cursor();
    let mut receiver = session.subscribe();
    let resumed: runs::ResumeRunResponse = match proxy
        .call(runs::methods::RESUME_RUN, (request.clone(),))
        .await
    {
        Ok(resumed) => resumed,
        Err(error) => {
            // A remote method error is a definitive ResumeRun rejection (a
            // stale-graph refusal may already have committed a terminal
            // event, which must still pass through PollRun). Transport and
            // decode errors are ambiguous: the module may have admitted the
            // actor before the acknowledgement was lost, so retain the fresh
            // callback and poll its durable cursor.
            let uncertain = !matches!(&error, tinybus::Error::MethodFailed { .. });
            if uncertain {
                session.replace_flow(candidate.flow.clone()).await;
                session.replace_bindings(Arc::clone(&candidate));
            }
            let terminal = session.drain(&proxy, true).await.unwrap_or(false);
            if terminal || (inserted && !uncertain) {
                session::remove(&session);
            } else if inserted || uncertain {
                session.start_polling(proxy)?;
            }
            return Err(error.wire_message());
        }
    };
    if resumed.run_id != request.run_id {
        if inserted {
            session::remove(&session);
        }
        return Err("TinyFlows resumed a different run id".to_string());
    }

    session.replace_flow(candidate.flow.clone()).await;
    session.set_trigger(FlowRunTrigger::Resume);
    session.mark_resumed();
    session.replace_bindings(candidate);
    session.start_polling(proxy)?;
    wait_for_outcome(&mut receiver, &request.run_id, baseline).await
}

/// Cancel a run through its exact owner-scoped CatalogStorage callback.
pub(crate) async fn cancel_run(
    config: &Config,
    storage: tinyflows_bus::catalog::CatalogStorage,
    run_id: &str,
) -> Result<(bool, bool), String> {
    let active = session::get(&storage, run_id);
    let proxy = module::runs_proxy(config).await?;
    let response: runs::CancelRunResponse = proxy
        .call(
            runs::methods::CANCEL_RUN,
            (runs::RunControlRequest {
                storage,
                run_id: run_id.to_owned(),
                cursor: 0,
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    let was_in_flight = response.signalled
        && active
            .as_ref()
            .is_some_and(|session| session.is_live_actor());
    Ok((response.signalled, was_in_flight))
}

/// Expire parked runs in a scope, then drain their terminal events through the
/// same projector used by active sessions.
pub(crate) async fn expire_parked(
    ctx: &DispatchCtx,
    config: &Config,
    catalog: Arc<CatalogStorageRegistration>,
) -> Result<runs::ExpireParkedResponse, String> {
    let proxy = module::runs_proxy(config).await?;
    let response: runs::ExpireParkedResponse = proxy
        .call(
            runs::methods::EXPIRE_PARKED,
            (runs::ExpireParked {
                storage: catalog.storage(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    for (run_id, _) in &response.expired {
        after_recovery(&catalog, run_id, config).await?;
    }
    let _ = ctx;
    Ok(response)
}

/// Sweep orphaned runs in this scope and project only rows changed by the
/// module's winning compare-and-swap.
pub(crate) async fn boot_sweep(
    ctx: &DispatchCtx,
    config: &Config,
    catalog: Arc<CatalogStorageRegistration>,
) -> Result<runs::BootSweepResponse, String> {
    let proxy = module::runs_proxy(config).await?;
    let response: runs::BootSweepResponse = proxy
        .call(
            runs::methods::BOOT_SWEEP,
            (runs::BootSweep {
                storage: catalog.storage(),
                shared_backend: crate::storage::installed_is_shared(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    for run_id in &response.interrupted {
        after_recovery(&catalog, run_id, config).await?;
    }
    let _ = ctx;
    Ok(response)
}

/// Project a recovered terminal event without reconstructing run state in the
/// host. The storage address remains part of the dedupe key and data lookup.
pub(crate) async fn after_recovery(
    catalog: &Arc<CatalogStorageRegistration>,
    run_id: &str,
    config: &Config,
) -> Result<(), String> {
    let storage = catalog.storage();
    if let Some(session) = session::get(&storage, run_id) {
        let proxy = module::runs_proxy(config).await?;
        if session.drain(&proxy, true).await? {
            session::remove(&session);
        }
        return Ok(());
    }

    let key = RunKey::new(&storage, run_id);
    let runs_proxy = module::runs_proxy(config).await?;
    let catalog_proxy = module::catalog_proxy(config).await?;
    let mut cursor = 0;
    loop {
        let page: runs::PollRun = runs_proxy
            .call(
                runs::methods::POLL_RUN,
                (runs::RunControlRequest {
                    storage: storage.clone(),
                    run_id: run_id.to_owned(),
                    cursor,
                },),
            )
            .await
            .map_err(|error| error.wire_message())?;
        for event in &page.events {
            if event.kind == "finished" {
                let run: Option<tinyflows_bus::catalog::FlowRun> = catalog_proxy
                    .call(
                        tinyflows_bus::catalog::members::GET_RUN,
                        (tinyflows_bus::catalog::RunIdRequest {
                            storage: storage.clone(),
                            id: run_id.to_owned(),
                        },),
                    )
                    .await
                    .map_err(|error| error.wire_message())?;
                let flow_id = run
                    .ok_or_else(|| format!("flow run '{run_id}' disappeared during recovery"))?
                    .flow_id;
                projection::project_recovered_finished(&key, &flow_id, run_id, event);
            }
        }
        if page.done || page.events.is_empty() || page.next_cursor <= cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(())
}

fn finished_outcome(finished: runs::RunFinished, run_id: &str) -> Result<RunOutcome, String> {
    if finished.timed_out {
        return Err(finished
            .error
            .unwrap_or_else(|| "flow run timed out".to_string()));
    }
    if finished.status == "cancelled" {
        return Ok(RunOutcome {
            run_id: run_id.to_owned(),
            output: Value::Null,
            pending_approvals: Vec::new(),
            status: finished.status,
        });
    }
    let output = finished.output.ok_or_else(|| {
        finished.error.clone().unwrap_or_else(|| {
            format!(
                "flow run ended as {} without an engine outcome",
                finished.status
            )
        })
    })?;
    Ok(RunOutcome {
        run_id: run_id.to_owned(),
        output,
        pending_approvals: Vec::new(),
        status: finished.status,
    })
}

fn bus_trigger(trigger: FlowRunTrigger) -> runs::FlowRunTrigger {
    match trigger {
        FlowRunTrigger::Rpc => runs::FlowRunTrigger::Rpc,
        FlowRunTrigger::Schedule => runs::FlowRunTrigger::Schedule,
        FlowRunTrigger::AppEvent => runs::FlowRunTrigger::AppEvent,
        FlowRunTrigger::Resume => runs::FlowRunTrigger::Resume,
    }
}

#[cfg(test)]
#[path = "runs_adapter_tests.rs"]
mod tests;

async fn run_exists(
    config: &Config,
    storage: tinyflows_bus::catalog::CatalogStorage,
    run_id: &str,
) -> Result<bool, String> {
    let proxy = module::catalog_proxy(config).await?;
    let run: Option<tinyflows_bus::catalog::FlowRun> = proxy
        .call(
            tinyflows_bus::catalog::members::GET_RUN,
            (tinyflows_bus::catalog::RunIdRequest {
                storage,
                id: run_id.to_owned(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    Ok(run.is_some())
}
