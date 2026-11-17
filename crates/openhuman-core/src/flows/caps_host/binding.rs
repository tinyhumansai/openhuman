//! One trusted `CapsHost` callback object for a single TinyFlows run.

use std::future::Future;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::flows::catalog_storage::CatalogStorageRegistration;
use crate::modules::tinyflows as module;
use tinyflows_bus::runs::{CallbackAddress, CapsHost};

use super::{agent::AgentRunState, service::CapsHostService};

/// Owns the host callback and its dependent owner-scoped catalog callback.
/// Keep this handle alive until the module has settled or parked the run.
pub(crate) struct CapsHostRegistration {
    _registration: module::ServedObjectRegistration,
    _catalog: Arc<CatalogStorageRegistration>,
    _binding: Arc<CapsHostBinding>,
    address: CapsHost,
}

/// Request-scoped values shared by all capability callbacks for one run.
pub(crate) struct CapsHostBinding {
    pub(crate) ctx: DispatchCtx,
    pub(crate) config: Arc<Config>,
    pub(crate) flow_id: String,
    pub(crate) run_id: String,
    pub(crate) security: Arc<crate::security::SecurityPolicy>,
    pub(crate) approval: crate::security::approval::ApprovalTurnContext,
    pub(crate) cancellation: tokio_util::sync::CancellationToken,
    pub(crate) agent_runs: tokio::sync::Mutex<AgentRunRegistry>,
}

/// Bounds invocation records and cancel-before-start tombstones retained by
/// one run binding. IDs only correlate requests within this binding; the
/// callback address remains the authority boundary. Once capacity is exhausted,
/// new ids fail closed for the remaining lifetime of the binding.
const MAX_AGENT_RUN_RECORDS: usize = 10_000;
const MAX_AGENT_CANCEL_TOMBSTONES: usize = 4_096;

pub(crate) struct AgentRunEntry {
    pub(crate) handle: String,
    pub(crate) state: Arc<tokio::sync::Mutex<AgentRunState>>,
    pub(crate) cancellation: tokio_util::sync::CancellationToken,
}

pub(crate) struct AgentRunAdmission {
    pub(crate) entry: AgentRunEntry,
    pub(crate) start_task: bool,
    pub(crate) cancelled_before_admission: bool,
}

#[derive(Default)]
pub(crate) struct AgentRunRegistry {
    by_request_id: HashMap<String, AgentRunEntry>,
    by_handle: HashMap<String, Arc<tokio::sync::Mutex<AgentRunState>>>,
    cancelled_before_start: HashSet<String>,
    cancellation_order: VecDeque<String>,
    reject_new_ids: bool,
}

impl AgentRunRegistry {
    pub(crate) fn admit(
        &mut self,
        client_request_id: String,
        state: Arc<tokio::sync::Mutex<AgentRunState>>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<AgentRunAdmission, String> {
        if let Some(entry) = self.by_request_id.get(&client_request_id) {
            return Ok(AgentRunAdmission {
                entry: entry.clone(),
                start_task: false,
                cancelled_before_admission: false,
            });
        }
        if self.reject_new_ids {
            return Err("CapsHost agent invocation capacity is exhausted".to_owned());
        }
        if self.by_request_id.len() >= MAX_AGENT_RUN_RECORDS {
            self.reject_new_ids = true;
            return Err("CapsHost agent invocation capacity is exhausted".to_owned());
        }

        let cancelled_before_admission = self.cancelled_before_start.remove(&client_request_id);
        if cancelled_before_admission {
            self.cancellation_order
                .retain(|request_id| request_id != &client_request_id);
            cancellation.cancel();
        }
        let entry = AgentRunEntry {
            handle: uuid::Uuid::new_v4().simple().to_string(),
            state: Arc::clone(&state),
            cancellation,
        };
        self.by_handle.insert(entry.handle.clone(), state);
        self.by_request_id.insert(client_request_id, entry.clone());
        Ok(AgentRunAdmission {
            entry,
            start_task: !cancelled_before_admission,
            cancelled_before_admission,
        })
    }

    pub(crate) fn cancel(&mut self, client_request_id: String) {
        if let Some(entry) = self.by_request_id.get(&client_request_id) {
            entry.cancellation.cancel();
            return;
        }
        if self.cancelled_before_start.contains(&client_request_id) {
            return;
        }
        if self.cancelled_before_start.len() >= MAX_AGENT_CANCEL_TOMBSTONES {
            self.reject_new_ids = true;
            return;
        }
        self.cancelled_before_start
            .insert(client_request_id.clone());
        self.cancellation_order.push_back(client_request_id);
    }

    pub(crate) fn state_for_handle(
        &self,
        handle: &str,
    ) -> Option<Arc<tokio::sync::Mutex<AgentRunState>>> {
        self.by_handle.get(handle).cloned()
    }
}

impl Clone for AgentRunEntry {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            state: Arc::clone(&self.state),
            cancellation: self.cancellation.clone(),
        }
    }
}

impl Drop for CapsHostRegistration {
    fn drop(&mut self) {
        self._binding.cancellation.cancel();
    }
}

impl CapsHostBinding {
    /// Re-enter the request's captured tenant, identity and turn origin for a
    /// bus callback that runs on the module runtime.
    pub(crate) async fn within<F: Future>(&self, future: F) -> F::Output {
        match self.ctx.core() {
            Some(context) => {
                crate::core::runtime::CoreContext::scope(Arc::clone(context), future).await
            }
            None => future.await,
        }
    }
}

impl CapsHostRegistration {
    /// Address to pass unchanged to `Runs.StartRun` or `Runs.ResumeRun`.
    pub(crate) fn address(&self) -> CapsHost {
        self.address.clone()
    }
}

/// Bind and serve all capability callbacks for one run generation.
pub(crate) async fn serve(
    ctx: DispatchCtx,
    config: Arc<Config>,
    flow_id: String,
    run_id: String,
    require_approval: bool,
    catalog: Arc<CatalogStorageRegistration>,
) -> Result<CapsHostRegistration, String> {
    let origin = crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.clone(),
        source: crate::core::runtime::turn_origin::TrustedAutomationSource::Workflow {
            require_approval,
        },
    };
    let ctx = ctx.with_turn_origin(origin);
    let security = Arc::new(crate::security::SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let approval = crate::security::approval::ApprovalTurnContext {
        flow_run: Some(crate::security::approval::FlowRunContext {
            flow_id: flow_id.clone(),
            run_id: run_id.clone(),
        }),
        ..Default::default()
    };
    let binding = Arc::new(CapsHostBinding {
        ctx,
        config,
        flow_id,
        run_id,
        security,
        approval,
        cancellation: tokio_util::sync::CancellationToken::new(),
        agent_runs: tokio::sync::Mutex::new(AgentRunRegistry::default()),
    });
    let path = format!(
        "/ai/tinyhumans/openhuman/CapsHost/{}",
        uuid::Uuid::new_v4().simple()
    );
    let address = CapsHost {
        callback: CallbackAddress {
            destination: module::HOST_BUS_NAME.to_string(),
            object_path: path.clone(),
        },
    };
    let service = CapsHostService::new(Arc::clone(&binding));
    let registration = module::serve_registered(path, service).await?;
    Ok(CapsHostRegistration {
        _registration: registration,
        _catalog: catalog,
        _binding: binding,
        address,
    })
}
