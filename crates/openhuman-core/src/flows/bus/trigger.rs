//! Trigger → run bridge: [`FlowTriggerSubscriber`] listens for the
//! normalized events a saved flow's trigger node can bind to and spawns
//! `flows::ops::flows_run` for each match, plus the trigger-config readers
//! `flows::ops` reuses to bind/unbind automatic dispatch.

use crate::config::Config;
use crate::core::events::DomainEvent;
use crate::flows::store;
use crate::modules::tinyflows as module;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tinybus::EventHandler;
use tinyflows_bus::catalog::Workflow as Flow;
use tinyflows_bus::runs::FlowRunTrigger;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

fn portability_call<T, A>(config: &Config, member: &'static str, request: A) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
    A: Serialize + Send + Sync + 'static,
{
    let config = config.clone();
    crate::storage::block_on_anyhow(async move {
        let proxy = module::portability_proxy(&config)
            .await
            .map_err(anyhow::Error::msg)?;
        proxy
            .call(member, &(request,))
            .await
            .map_err(anyhow::Error::msg)
    })
    .map_err(|error| error.to_string())
}

/// Ask TinyFlows for normalized trigger metadata used by host bindings.
pub(crate) fn describe_trigger(
    config: &Config,
    flow: &Flow,
) -> Result<tinyflows_bus::portability::TriggerDetails, String> {
    let graph = flow.graph.clone();
    portability_call(
        config,
        tinyflows_bus::portability::methods::DESCRIBE_TRIGGER,
        tinyflows_bus::portability::DescribeTrigger { graph },
    )
}

/// Return author-pinned literal inputs for an unattended trigger run.
pub(super) fn pinned_trigger_inputs(
    config: &Config,
    flow: &Flow,
) -> Result<serde_json::Map<String, Value>, String> {
    let graph = flow.graph.clone();
    let value: Value = portability_call(
        config,
        tinyflows_bus::portability::methods::PINNED_INPUTS,
        tinyflows_bus::portability::PinnedInputs { graph },
    )?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "TinyFlows returned pinned inputs in an invalid shape".to_string())
}

/// True when `flow` is an enabled `app_event` flow bound to the given
/// Composio `toolkit`/`trigger_slug` (case-insensitive — Composio slugs are
/// conventionally upper-case but authoring surfaces may not normalize them).
pub(super) fn matches_app_event(
    config: &Config,
    flow: &Flow,
    toolkit: &str,
    trigger_slug: &str,
) -> Result<bool, String> {
    let graph = flow.graph.clone();
    portability_call(
        config,
        tinyflows_bus::portability::methods::APP_EVENT_BINDING,
        tinyflows_bus::portability::AppEventBinding {
            graph,
            toolkit: toolkit.to_string(),
            trigger_slug: trigger_slug.to_string(),
        },
    )
}

/// Listens for normalized trigger events and starts runs for matching
/// enabled flows. See the module doc for the full contract.
pub struct FlowTriggerSubscriber {
    config: Arc<RwLock<Config>>,
    queues: Arc<Mutex<HashMap<DispatchKey, Arc<FlowRunQueue>>>>,
    runner: Arc<RunExecutor>,
    queue_capacity: usize,
    #[cfg(test)]
    retiring: Arc<tokio::sync::Notify>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DispatchKey {
    tenant: crate::core::runtime::Tenant,
    flow_id: String,
}

struct FlowRunQueue {
    sender: mpsc::Sender<QueuedRun>,
    receiver: Mutex<Option<mpsc::Receiver<QueuedRun>>>,
    worker_started: std::sync::atomic::AtomicBool,
    admission: Arc<AsyncMutex<()>>,
    outstanding: AtomicUsize,
}

struct QueuedRun {
    ctx: crate::core::runtime::DispatchCtx,
    config: Arc<Config>,
    request: TriggerRunRequest,
}

struct QueueOutstanding {
    queue: Arc<FlowRunQueue>,
    armed: bool,
}

impl QueueOutstanding {
    fn new(queue: Arc<FlowRunQueue>) -> Self {
        queue.outstanding.fetch_add(1, Ordering::AcqRel);
        Self { queue, armed: true }
    }

    fn transfer_to_worker(mut self) {
        self.armed = false;
    }
}

impl Drop for QueueOutstanding {
    fn drop(&mut self) {
        if self.armed {
            self.queue.outstanding.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

pub(crate) struct TriggerRunRequest {
    pub(crate) flow_id: String,
    pub(crate) input: Value,
    pub(crate) inputs: serde_json::Map<String, Value>,
    pub(crate) trigger: FlowRunTrigger,
}

type RunFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type RunExecutor = dyn Fn(crate::core::runtime::DispatchCtx, Arc<Config>, TriggerRunRequest) -> RunFuture
    + Send
    + Sync;

const APP_EVENT_QUEUE_CAPACITY: usize = 32;

impl FlowTriggerSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self::with_config_source(Arc::new(RwLock::new((*config).clone())))
    }

    pub(crate) fn with_config_source(config: Arc<RwLock<Config>>) -> Self {
        Self::with_executor(
            config,
            APP_EVENT_QUEUE_CAPACITY,
            |ctx, config, request| async move {
                let flow_id = request.flow_id.clone();
                tracing::info!(target: "flows", %flow_id, "[flows] trigger fired — starting run");
                match crate::flows::ops::flows_run(
                    &ctx,
                    &config,
                    &flow_id,
                    request.input,
                    request.inputs,
                    request.trigger,
                )
                .await
                {
                    Ok(_) => {
                        tracing::info!(target: "flows", %flow_id, "[flows] trigger-driven run finished")
                    }
                    Err(e) => {
                        tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] trigger-driven run failed")
                    }
                }
            },
        )
    }

    fn with_executor<F, Fut>(config: Arc<RwLock<Config>>, queue_capacity: usize, runner: F) -> Self
    where
        F: Fn(crate::core::runtime::DispatchCtx, Arc<Config>, TriggerRunRequest) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let runner: Arc<RunExecutor> =
            Arc::new(move |ctx, config, request| Box::pin(runner(ctx, config, request)));
        Self {
            config,
            queues: Arc::new(Mutex::new(HashMap::new())),
            runner,
            queue_capacity: queue_capacity.max(1),
            #[cfg(test)]
            retiring: Arc::new(tokio::sync::Notify::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_runner<F, Fut>(config: Arc<Config>, queue_capacity: usize, runner: F) -> Self
    where
        F: Fn(crate::core::runtime::DispatchCtx, Arc<Config>, TriggerRunRequest) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::with_executor(
            Arc::new(RwLock::new((*config).clone())),
            queue_capacity,
            runner,
        )
    }

    #[cfg(test)]
    pub(crate) fn with_runner_and_config_source<F, Fut>(
        config: Arc<RwLock<Config>>,
        queue_capacity: usize,
        runner: F,
    ) -> Self
    where
        F: Fn(crate::core::runtime::DispatchCtx, Arc<Config>, TriggerRunRequest) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::with_executor(config, queue_capacity, runner)
    }

    async fn dispatch_app_event(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: String,
        payload: Value,
        inputs: serde_json::Map<String, Value>,
    ) {
        // Admission intentionally waits for bounded per-key queue capacity.
        // EventHandler runs inline in TinyBus, so a saturated key can delay
        // later scopes in this callback. This is in-memory backpressure, not
        // durable delivery; the upstream event broadcast is itself bounded.
        let Some(key) = self.dispatch_key(ctx, &flow_id) else {
            return;
        };
        let queued = QueuedRun {
            ctx: ctx.clone(),
            config: self.config_for_scope(ctx),
            request: TriggerRunRequest {
                flow_id,
                input: payload,
                inputs,
                trigger: FlowRunTrigger::AppEvent,
            },
        };
        self.enqueue_app_event(key, ctx, queued).await;
    }

    #[cfg(test)]
    pub(crate) async fn dispatch_test_app_event(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
        payload: Value,
    ) {
        self.dispatch_app_event(ctx, flow_id.to_string(), payload, serde_json::Map::new())
            .await;
    }

    fn dispatch_key(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) -> Option<DispatchKey> {
        match ctx.tenant() {
            Ok(tenant) => Some(DispatchKey {
                tenant,
                flow_id: flow_id.to_string(),
            }),
            Err(error) => {
                tracing::warn!(target: "flows", %flow_id, error = %error, "[flows] trigger dispatch has no tenant scope; refusing to use the host config");
                None
            }
        }
    }

    fn queue_for(&self, key: &DispatchKey) -> Arc<FlowRunQueue> {
        let mut queues = self
            .queues
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(queue) = queues.get(key).filter(|queue| !queue.sender.is_closed()) {
            return Arc::clone(queue);
        }

        let (sender, receiver) = mpsc::channel(self.queue_capacity);
        let queue = Arc::new(FlowRunQueue {
            sender,
            receiver: Mutex::new(Some(receiver)),
            worker_started: std::sync::atomic::AtomicBool::new(false),
            admission: Arc::new(AsyncMutex::new(())),
            outstanding: AtomicUsize::new(0),
        });
        queues.insert(key.clone(), Arc::clone(&queue));
        queue
    }

    fn start_worker_if_needed(
        &self,
        key: &DispatchKey,
        queue: &Arc<FlowRunQueue>,
        ctx: &crate::core::runtime::DispatchCtx,
    ) {
        if queue
            .worker_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let Some(receiver) = queue
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        else {
            return;
        };
        let worker_key = key.clone();
        let worker_queue = Arc::clone(queue);
        let worker_queues = Arc::clone(&self.queues);
        let runner = Arc::clone(&self.runner);
        #[cfg(test)]
        let retiring = Arc::clone(&self.retiring);
        ctx.spawn(async move {
            run_queue_worker(
                worker_key,
                worker_queue,
                receiver,
                worker_queues,
                runner,
                #[cfg(test)]
                retiring,
            )
            .await;
        });
    }

    async fn enqueue_app_event(
        &self,
        key: DispatchKey,
        ctx: &crate::core::runtime::DispatchCtx,
        mut queued: QueuedRun,
    ) {
        loop {
            let queue = self.queue_for(&key);
            let admission = Arc::clone(&queue.admission).lock_owned().await;
            if queue.sender.is_closed() {
                drop(admission);
                self.remove_queue_if_current(&key, &queue);
                continue;
            }

            let outstanding = QueueOutstanding::new(Arc::clone(&queue));
            match queue.sender.send(queued).await {
                Ok(()) => {
                    outstanding.transfer_to_worker();
                    drop(admission);
                    self.start_worker_if_needed(&key, &queue, ctx);
                    return;
                }
                Err(error) => {
                    queued = error.0;
                    drop(outstanding);
                    drop(admission);
                    self.remove_queue_if_current(&key, &queue);
                }
            }
        }
    }

    async fn enqueue_schedule(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: String,
        inputs: serde_json::Map<String, Value>,
    ) {
        let Some(key) = self.dispatch_key(ctx, &flow_id) else {
            return;
        };
        let queued = QueuedRun {
            ctx: ctx.clone(),
            config: self.config_for_scope(ctx),
            request: TriggerRunRequest {
                flow_id: flow_id.clone(),
                input: Value::Null,
                inputs,
                trigger: FlowRunTrigger::Schedule,
            },
        };
        self.enqueue_schedule_request(key, ctx, queued).await;
    }

    async fn enqueue_schedule_request(
        &self,
        key: DispatchKey,
        ctx: &crate::core::runtime::DispatchCtx,
        mut queued: QueuedRun,
    ) {
        loop {
            let queue = self.queue_for(&key);
            let admission = Arc::clone(&queue.admission).lock_owned().await;
            if queue.sender.is_closed() {
                drop(admission);
                self.remove_queue_if_current(&key, &queue);
                continue;
            }
            if queue.outstanding.load(Ordering::Acquire) != 0 {
                tracing::debug!(target: "flows", flow_id = %key.flow_id, "[flows] schedule tick coalesced while a trigger run is queued or active");
                return;
            }

            queue.outstanding.fetch_add(1, Ordering::AcqRel);
            match queue.sender.try_send(queued) {
                Ok(()) => {
                    drop(admission);
                    self.start_worker_if_needed(&key, &queue, ctx);
                    return;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    queue.outstanding.fetch_sub(1, Ordering::AcqRel);
                    tracing::debug!(target: "flows", flow_id = %key.flow_id, "[flows] schedule tick coalesced while a trigger run is queued");
                    return;
                }
                Err(mpsc::error::TrySendError::Closed(returned)) => {
                    queue.outstanding.fetch_sub(1, Ordering::AcqRel);
                    drop(admission);
                    self.remove_queue_if_current(&key, &queue);
                    // Keep the value available if the retiring worker won
                    // the race with this schedule tick.
                    queued = returned;
                }
            }
        }
    }

    fn remove_queue_if_current(&self, key: &DispatchKey, queue: &Arc<FlowRunQueue>) {
        let mut queues = self
            .queues
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if queues
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, queue))
        {
            queues.remove(key);
        }
    }

    /// The configuration to handle an event under (`super::owner::config_for_scope`).
    pub(super) fn config_for_scope(&self, ctx: &crate::core::runtime::DispatchCtx) -> Arc<Config> {
        let config = Arc::new(
            self.config
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        );
        super::owner::config_for_scope(ctx, &config)
    }

    /// A `flow`-type cron job fired (`cron::scheduler`'s trigger sink calls
    /// this directly). Runs as the agent the flow belongs to
    /// (`super::owner`), then loads the one named flow, checks it is still
    /// enabled with a `schedule` trigger (it may have been disabled/edited
    /// since the job was registered), and dispatches it with an empty trigger
    /// payload.
    pub(crate) async fn dispatch_schedule_tick(
        &self,
        host: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) {
        let config = self.config_for_scope(host);
        super::owner::as_owner(host, &config, flow_id, |ctx| {
            self.handle_schedule_tick(ctx, flow_id)
        })
        .await
    }

    async fn handle_schedule_tick(&self, ctx: crate::core::runtime::DispatchCtx, flow_id: &str) {
        let config = self.config_for_scope(&ctx);
        let flow = match store::get_flow(&config, flow_id) {
            Ok(Some(flow)) => flow,
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for unknown/removed flow — ignoring");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] failed to load flow for schedule tick");
                return;
            }
        };
        if !flow.enabled {
            tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for disabled flow — ignoring");
            return;
        }
        let details = match describe_trigger(&config, &flow) {
            Ok(details) => details,
            Err(error) => {
                tracing::warn!(target: "flows", %flow_id, %error, "[flows] failed to describe trigger for schedule tick");
                return;
            }
        };
        if details.kind.as_deref() != Some("schedule") {
            tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for flow whose trigger is no longer `schedule` — ignoring");
            return;
        }
        let inputs = match pinned_trigger_inputs(&config, &flow) {
            Ok(inputs) => inputs,
            Err(error) => {
                tracing::warn!(target: "flows", %flow_id, %error, "[flows] failed to read pinned inputs for schedule trigger");
                return;
            }
        };
        self.enqueue_schedule(&ctx, flow_id.to_string(), inputs)
            .await;
    }

    /// `DomainEvent::ComposioTriggerReceived` — scans every enabled flow for
    /// an `app_event` trigger bound to this `toolkit`/`trigger_slug` and
    /// dispatches each match with the event payload as the run input
    /// (seeded into `run.trigger`, per the node-catalog contract).
    async fn handle_app_event(
        &self,
        ctx: crate::core::runtime::DispatchCtx,
        toolkit: &str,
        trigger_slug: &str,
        payload: &Value,
    ) {
        let config = self.config_for_scope(&ctx);
        let (flows, skipped) = match store::list_enabled_flows(&config) {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(target: "flows", %toolkit, %trigger_slug, error = %e, "[flows] failed to list enabled flows for app_event dispatch");
                return;
            }
        };
        if skipped > 0 {
            // R-M4: one corrupt/unmigratable flow row must not blackhole
            // app_event dispatch for every other enabled flow.
            tracing::warn!(target: "flows", %toolkit, %trigger_slug, skipped, "[flows] handle_app_event: skipped corrupt/unmigratable flow rows while matching trigger");
        }

        let mut matched = 0usize;
        for flow in flows {
            let matches = match matches_app_event(&config, &flow, toolkit, trigger_slug) {
                Ok(matches) => matches,
                Err(error) => {
                    tracing::warn!(target: "flows", flow_id = %flow.id, %toolkit, %trigger_slug, %error, "[flows] failed to match app_event trigger");
                    continue;
                }
            };
            if matches {
                matched += 1;
                let inputs = match pinned_trigger_inputs(&config, &flow) {
                    Ok(inputs) => inputs,
                    Err(error) => {
                        tracing::warn!(target: "flows", flow_id = %flow.id, %error, "[flows] failed to read pinned inputs for app_event trigger");
                        continue;
                    }
                };
                self.dispatch_app_event(&ctx, flow.id.clone(), payload.clone(), inputs)
                    .await;
            }
        }
        tracing::debug!(target: "flows", %toolkit, %trigger_slug, matched, "[flows] app_event trigger matching complete");
    }

    #[cfg(test)]
    pub(crate) async fn dispatch_test_schedule(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) {
        self.enqueue_schedule(ctx, flow_id.to_string(), serde_json::Map::new())
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn hold_queue_admission_test(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let key = self.dispatch_key(ctx, flow_id).unwrap();
        let queue = self.queue_for(&key);
        Arc::clone(&queue.admission).lock_owned().await
    }

    #[cfg(test)]
    pub(crate) fn retirement_notifier_test(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.retiring)
    }
}

async fn run_queue_worker(
    key: DispatchKey,
    queue: Arc<FlowRunQueue>,
    mut receiver: mpsc::Receiver<QueuedRun>,
    queues: Arc<Mutex<HashMap<DispatchKey, Arc<FlowRunQueue>>>>,
    runner: Arc<RunExecutor>,
    #[cfg(test)] retiring: Arc<tokio::sync::Notify>,
) {
    let mut next = receiver.recv().await;
    loop {
        let Some(queued) = next.take() else {
            break;
        };
        let run_ctx = queued.ctx.clone();
        let flow_id = queued.request.flow_id.clone();
        let run = run_ctx.spawn((runner)(queued.ctx, queued.config, queued.request));
        if let Err(error) = run.await {
            tracing::warn!(target: "flows", %flow_id, error = %error, "[flows] trigger run task exited unexpectedly");
        }
        queue.outstanding.fetch_sub(1, Ordering::AcqRel);

        if let Ok(queued) = receiver.try_recv() {
            next = Some(queued);
            continue;
        }

        // A producer takes the same gate before sending. Once this worker
        // holds it and observes an empty receiver, no successful send can be
        // stranded behind a closed receiver. A sender that cloned this slot
        // before retirement gets SendError and retries with the new slot.
        #[cfg(test)]
        retiring.notify_one();
        let admission = Arc::clone(&queue.admission).lock_owned().await;
        match receiver.try_recv() {
            Ok(queued) => {
                drop(admission);
                next = Some(queued);
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                receiver.close();
                let mut current = queues.lock().unwrap_or_else(|error| error.into_inner());
                if current
                    .get(&key)
                    .is_some_and(|entry| Arc::ptr_eq(entry, &queue))
                {
                    current.remove(&key);
                }
                return;
            }
        }
    }

    let mut current = queues.lock().unwrap_or_else(|error| error.into_inner());
    if current
        .get(&key)
        .is_some_and(|entry| Arc::ptr_eq(entry, &queue))
    {
        current.remove(&key);
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for FlowTriggerSubscriber {
    fn name(&self) -> &str {
        "flows::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron", "composio", "webhook", "system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // Trigger events arrive over the host's socket or its own scheduler:
        // the host's own tenant, and in SaaS none.
        let host = crate::core::runtime::DispatchCtx::host_default();
        match event {
            // Every scope's flows may listen for this trigger: match them in
            // each, so each run starts as its flow's owner.
            DomainEvent::ComposioTriggerReceived {
                toolkit,
                trigger,
                payload,
                ..
            } => {
                crate::storage::agents::for_each_live_scope(&host, "flows app_event", |ctx| {
                    self.handle_app_event(ctx, toolkit, trigger, payload)
                })
                .await;
            }
            DomainEvent::WebhookIncomingRequest { .. } => {
                // Best-effort deviation (documented, not silently skipped —
                // see `flows::ops::log_webhook_trigger_deferred` for the
                // enable/disable-side note): a `webhook`-trigger flow needs a
                // backend-provisioned tunnel + a UI surface for the resulting
                // URL, neither of which exists yet. Never log the request's
                // `raw_data` here — it is untrusted, possibly-sensitive
                // inbound payload.
                tracing::debug!(
                    target: "flows",
                    "[flows] observed WebhookIncomingRequest — webhook-trigger dispatch is not \
                     implemented in B2 (pending backend tunnel provisioning + B3 UI); no flow \
                     dispatched"
                );
            }
            other => {
                // Anything else on our filtered domains (plain shell/agent
                // `CronJobTriggered`, other Composio lifecycle events,
                // system lifecycle, …) is not a flow trigger — ignore. Log
                // only the variant name, never the event's Debug form: some
                // sibling variants on these domains carry payloads we must
                // not put in logs (e.g. `ComposioTriggerReceived::payload`).
                tracing::trace!(target: "flows", variant = other.variant_name(), "[flows] ignoring unrelated event");
            }
        }
    }
}
