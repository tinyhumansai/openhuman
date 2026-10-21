use super::*;
#[cfg(test)]
use std::path::PathBuf;
use tinyflows_bus::schedule::{
    AcknowledgeManualRun, ManualRunHandle, ManualRunOutcome, ManualRunRequest, ManualRunState,
    PrepareManualRun, StartManualRun, WaitManualRun,
};

/// Compatibility adapter for callers that need a synchronous Run Now result.
/// TinyFlows owns admission, retries, delivery and persistence.
pub async fn execute_job_now(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
) -> (bool, String) {
    match run_job_now(
        ctx,
        config,
        job.clone(),
        run_id.to_string(),
        true,
        ManualDelivery::Requested,
    )
    .await
    {
        Ok(RunJobReply::Completed {
            success, attempt, ..
        }) => (success, attempt.output),
        Ok(RunJobReply::Refused { reason }) => (false, format!("cron job refused: {reason:?}")),
        Ok(RunJobReply::Accepted { .. }) => (
            false,
            "cron module accepted a synchronous run without completing it".to_string(),
        ),
        Err(error) => {
            tracing::debug!(job_id = %job.id, %run_id, %error, "[cron:scheduler] RunJobNow failed");
            (false, error.to_string())
        }
    }
}

/// Start one invocation-owned manual worker. Its owner task is independent of
/// the RPC/tool future and keeps this invocation's callback and context alive
/// until the module reports terminal state and acknowledges the handle.
pub(crate) async fn run_job_now(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: CronJob,
    run_id: String,
    wait: bool,
    delivery: ManualDelivery,
) -> Result<RunJobReply> {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let ctx = ctx.clone();
    let config = config.clone();
    spawn_manual_owner(async move {
        let mut reply_tx = Some(reply_tx);
        let result = own_manual_run(ctx, config, job, run_id, wait, delivery, &mut reply_tx).await;
        if let Err(error) = result {
            if let Some(reply_tx) = reply_tx.take() {
                let _ = reply_tx.send(Err(error.to_string()));
            }
            tracing::warn!(%error, "[cron:scheduler] manual worker owner stopped before acknowledgement");
        }
    })?;
    reply_rx
        .await
        .map_err(|_| anyhow!("cron manual worker owner stopped before reporting admission"))?
        .map_err(anyhow::Error::msg)
}

async fn own_manual_run(
    ctx: crate::core::runtime::DispatchCtx,
    config: Config,
    job: CronJob,
    run_id: String,
    wait: bool,
    delivery: ManualDelivery,
    reply_tx: &mut Option<tokio::sync::oneshot::Sender<Result<RunJobReply, String>>>,
) -> Result<()> {
    // Keep the invocation's CoreContext strongly pinned while Binding itself
    // retains only a Weak reference. The task is the acyclic lifetime owner.
    let _context_pin = ctx.core().cloned();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let binding = Binding::new(&ctx, &config, security);
    let host = bus_host::register(binding.clone())
        .await
        .map_err(|e| anyhow!(e))?;
    let scope = binding.within(async { DocScope::current(&config) }).await?;
    let storage = StoreConfig {
        backend: StoreBackend::Documents {
            callback: crate::cron::documents_port::address(&scope).await?,
        },
        max_run_history: config.cron.max_run_history,
        max_tasks: config.scheduler.max_tasks,
    };
    // ManualStart bypasses ScheduleStore, so migration must precede Prepare and
    // any possible claim just as it does for the ordinary scheduler.
    crate::cron::client::store_in::<serde_json::Value>(
        &config,
        &scope,
        tinyflows_bus::schedule::StoreOperation::Migrate,
    )
    .await?;

    let scope_id = scope.scheduler_scope();
    let request_id = uuid::Uuid::new_v4().to_string();
    let prepare = PrepareManualRun {
        scope: scope_id,
        request_id,
    };
    let handle: ManualRunHandle = loop {
        let proxy = match module::proxy(&config).await {
            Ok(proxy) => proxy,
            Err(error) => {
                tracing::debug!(%error, "[cron:scheduler] retrying manual PrepareManualRun");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        match proxy.call(members::PREPARE_MANUAL_RUN, (&prepare,)).await {
            Ok(handle) => break handle,
            Err(error) => {
                tracing::debug!(error = %error.wire_message(), "[cron:scheduler] retrying manual PrepareManualRun");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    };
    let start = StartManualRun {
        handle: handle.clone(),
        request: ManualRunRequest {
            callback: host.address(),
            config: scheduler_config(&config),
            storage,
            job,
            run_id,
            delivery,
        },
    };

    let mut started = false;
    let mut retry_delay = Duration::from_millis(20);
    let terminal = loop {
        let state = if !started {
            match module::proxy(&config).await {
                Ok(proxy) => match proxy
                    .call::<ManualRunState>(members::START_MANUAL_RUN, (&start,))
                    .await
                {
                    Ok(state) => state,
                    Err(error) => {
                        tracing::debug!(error = %error.wire_message(), "[cron:scheduler] retrying identical StartManualRun");
                        tokio::time::sleep(retry_delay).await;
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                        continue;
                    }
                },
                Err(error) => {
                    tracing::debug!(%error, "[cron:scheduler] retrying identical StartManualRun");
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                    continue;
                }
            }
        } else {
            match module::proxy(&config).await {
                Err(error) => {
                    tracing::debug!(%error, "[cron:scheduler] retrying WaitManualRun");
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                    continue;
                }
                Ok(proxy) => match proxy
                    .call::<Option<ManualRunState>>(
                        members::WAIT_MANUAL_RUN,
                        (WaitManualRun {
                            handle: handle.clone(),
                        },),
                    )
                    .await
                {
                    Ok(Some(state)) => state,
                    // The released contract defines absence as no retained worker
                    // in this module generation. We cannot infer that the
                    // service is still in the generation that prepared this
                    // handle, so keep the callback and context and reconcile.
                    Ok(None) => {
                        if let Some(reply_tx) = reply_tx.take() {
                            let _ = reply_tx.send(Err(
                                "cron manual worker disappeared before a terminal result"
                                    .to_string(),
                            ));
                        }
                        tracing::debug!("[cron:scheduler] manual handle absent without observed terminal state; retaining callback while reconciling");
                        tokio::time::sleep(retry_delay).await;
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                        continue;
                    }
                    Err(error) => {
                        tracing::debug!(error = %error.wire_message(), "[cron:scheduler] retrying WaitManualRun");
                        tokio::time::sleep(retry_delay).await;
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                        continue;
                    }
                },
            }
        };

        match state {
            ManualRunState::Prepared => {
                started = false;
                retry_delay = Duration::from_millis(20);
                continue;
            }
            ManualRunState::Starting => {
                // Starting does not mean the claim or queue row is durable.
                started = true;
            }
            ManualRunState::Running { run_id } => {
                started = true;
                if !wait {
                    if let Some(reply_tx) = reply_tx.take() {
                        let _ = reply_tx.send(Ok(RunJobReply::Accepted { run_id }));
                    }
                }
            }
            ManualRunState::Terminal { outcome } => break outcome,
            ManualRunState::Stopping { .. } | ManualRunState::CleanupPending { .. } => {
                started = true;
            }
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
    };

    let outcome_reply = match terminal {
        ManualRunOutcome::Completed {
            run_id,
            attempt,
            success,
            delivery_status,
        } => Ok(RunJobReply::Completed {
            run_id,
            attempt,
            success,
            delivery_status,
        }),
        ManualRunOutcome::Refused { reason, .. } => Ok(RunJobReply::Refused { reason }),
        ManualRunOutcome::Stopped { .. } => Err("cron manual run was stopped".to_string()),
        ManualRunOutcome::Failed { message, .. } => Err(message),
    };
    if let Some(reply_tx) = reply_tx.take() {
        let _ = reply_tx.send(outcome_reply);
    }

    let ack = AcknowledgeManualRun { handle };
    let mut retry_delay = Duration::from_millis(20);
    loop {
        let result = match module::proxy(&config).await {
            Ok(proxy) => proxy
                .call::<bool>(members::ACKNOWLEDGE_MANUAL_RUN, (&ack,))
                .await
                .map_err(|error| error.wire_message()),
            Err(error) => {
                tracing::debug!(%error, "[cron:scheduler] retrying AcknowledgeManualRun");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                continue;
            }
        };
        #[cfg(test)]
        let result = if result.as_ref().is_ok_and(|acknowledged| *acknowledged)
            && wait_for_ack_reply_loss_test_gate(&config.workspace_dir).await
        {
            Err("simulated lost AcknowledgeManualRun reply after commit".to_string())
        } else {
            result
        };
        match result {
            Ok(true) => break,
            Ok(false) => {
                tracing::debug!("[cron:scheduler] terminal manual handle not yet acknowledged");
                match module::proxy(&config).await {
                    Ok(proxy) => match proxy
                        .call::<Option<ManualRunState>>(
                            members::WAIT_MANUAL_RUN,
                            (WaitManualRun {
                                handle: ack.handle.clone(),
                            },),
                        )
                        .await
                    {
                        // The worker's terminal state was observed before
                        // Ack, so absence now means the acknowledgement's
                        // effect committed despite an earlier lost response.
                        Ok(None) => break,
                        Ok(Some(ManualRunState::Terminal { .. })) => {}
                        Ok(Some(_)) => tracing::warn!("[cron:scheduler] acknowledged manual worker returned to a nonterminal state"),
                        Err(error) => tracing::debug!(error = %error.wire_message(), "[cron:scheduler] reconciling manual acknowledgement"),
                    },
                    Err(error) => tracing::debug!(%error, "[cron:scheduler] reconciling manual acknowledgement"),
                }
            }
            Err(error) => {
                tracing::debug!(%error, "[cron:scheduler] retrying ambiguous AcknowledgeManualRun")
            }
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
    }
    drop(host);
    Ok(())
}

#[cfg(test)]
struct AckReplyLossTestGate {
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

#[cfg(test)]
static ACK_REPLY_LOSS_TEST_GATES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<AckReplyLossTestGate>>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(test)]
pub(crate) fn lose_next_ack_reply_after_commit_for_test(
    workspace: PathBuf,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    ACK_REPLY_LOSS_TEST_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            workspace,
            Arc::new(AckReplyLossTestGate {
                entered: std::sync::Mutex::new(Some(entered_tx)),
                release: tokio::sync::Mutex::new(Some(release_rx)),
            }),
        );
    (entered_rx, release_tx)
}

#[cfg(test)]
async fn wait_for_ack_reply_loss_test_gate(workspace: &std::path::Path) -> bool {
    let gate = ACK_REPLY_LOSS_TEST_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(workspace);
    let Some(gate) = gate else {
        return false;
    };
    if let Some(entered) = gate
        .entered
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        let _ = entered.send(());
    }
    let release = gate.release.lock().await.take();
    if let Some(release) = release {
        let _ = release.await;
    }
    true
}

/// Single poll cycle of the scheduler loop, extracted so tests can drive
/// it without owning `tokio::time::interval`.
///
/// Emits a `scheduler` health signal in two cases:
/// - Poll itself failed (DB read) → `healthy: false` with the DB error.
/// - Poll succeeded, queue empty or not → `healthy: true` (#3312
///   recovery signal). Without this, a single transient job failure
///   that flipped the component to `error` by a dispatched job would stay
///   there indefinitely while the queue was idle — the health endpoint would
///   keep returning 503 and Docker would mark the container `unhealthy` for
///   hours until a manual restart.
/// - Per-job results (published by each job's delivery) continue to flip the
///   component back to `healthy: false` on a failure; the next tick that
///   survives the DB read will re-flip it to `healthy: true`.
pub(crate) async fn tick_once(
    config: &Config,
    last_emitted_health: &mut Option<bool>,
    dispatcher: &mut JobDispatcher,
) {
    tracing::debug!("[cron:scheduler] tick poll begin");
    let dispatched = match dispatcher.tick(config).await {
        Ok(count) => count,
        Err(e) => {
            tracing::warn!("[cron:scheduler] tick poll failed: {e}");
            // Transition-only emission: repeat failures stay quiet so
            // subscribers don't see an event-storm during a long outage.
            if *last_emitted_health != Some(false) {
                BUS.publish(DomainEvent::HealthChanged {
                    component: "scheduler".to_string(),
                    healthy: false,
                    message: Some(e.to_string()),
                });
                *last_emitted_health = Some(false);
            }
            return;
        }
    };

    // Transition-only emission for the recovery signal: the very first
    // successful tick after boot (or after a failure) fires; later successful
    // ticks are no-ops on the wire.
    if *last_emitted_health != Some(true) {
        tracing::debug!(
            "[cron:scheduler] tick poll ok dispatched={dispatched} (recovery signal: healthy=true)"
        );
        BUS.publish(DomainEvent::HealthChanged {
            component: "scheduler".to_string(),
            healthy: true,
            message: None,
        });
        *last_emitted_health = Some(true);
    } else {
        tracing::trace!(
            "[cron:scheduler] tick poll ok dispatched={dispatched} (steady state, no event)"
        );
    }

    if dispatched > 0 {
        // A dispatched job publishes `healthy: false` on failure directly on
        // the bus, without touching this tracker. Reset so the next
        // successful tick is again a transition and re-emits `healthy: true`.
        *last_emitted_health = None;
    }
}

/// One scope's scheduler: a manual-tick scheduler in the module, polled by the
/// host, dispatching under its context. Stopped when dropped.
pub(crate) struct JobDispatcher {
    binding: Binding,
    started: Option<Started>,
}

/// A scheduler running in the module, and the host object it calls back.
struct Started {
    handle: String,
    host: HostRegistration,
}

impl JobDispatcher {
    /// A dispatcher for the context `ctx` carries. Nothing is started until the
    /// first tick.
    pub(crate) fn new(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        security: &Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            binding: Binding::new(ctx, config, Arc::clone(security)),
            started: None,
        }
    }

    /// Restrict the dispatcher to agent jobs (the hosted-profile sweep).
    #[must_use]
    pub(crate) fn agent_only(mut self) -> Self {
        self.binding = self.binding.clone().agent_only();
        self
    }

    /// The dispatch context the scheduler's callbacks act under.
    #[cfg(test)]
    pub(crate) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        self.binding.dispatch_ctx()
    }

    async fn start(&self, config: &Config) -> Result<Started> {
        let host = bus_host::register(self.binding.clone())
            .await
            .map_err(|e| anyhow!(e))?;
        // The documents of the scope this dispatcher acts in: resolved under
        // its context, because the call below is made from a task that may
        // not carry it.
        let scope = self
            .binding
            .within(async { DocScope::current(config) })
            .await?;
        // StartScheduler bypasses the ScheduleStore wrapper in client.rs.
        // Run the module-owned schema migration on its exact document scope
        // before the first scheduler request can observe or claim jobs.
        let _: serde_json::Value = crate::cron::client::store_in(
            config,
            &scope,
            tinyflows_bus::schedule::StoreOperation::Migrate,
        )
        .await?;
        let storage = StoreConfig {
            backend: StoreBackend::Documents {
                callback: crate::cron::documents_port::address(&scope).await?,
            },
            max_run_history: config.cron.max_run_history,
            max_tasks: config.scheduler.max_tasks,
        };
        let request = StartScheduler {
            scope: scope.scheduler_scope(),
            callback: host.address(),
            config: scheduler_config(config),
            manual_ticks: true,
            storage: Some(storage),
        };
        let proxy = module::proxy(config).await.map_err(|e| anyhow!(e))?;
        let handle: String = proxy
            .call(members::START_SCHEDULER, (request,))
            .await
            .map_err(|error| anyhow!(error.wire_message()))?;
        tracing::debug!("[cron:scheduler] scheduler started in the tinyflows module");
        Ok(Started { handle, host })
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &mut self,
        config: &Config,
        member: &'static str,
    ) -> Result<R> {
        if self.started.is_none() {
            self.started = Some(self.start(config).await?);
        }
        let handle = self
            .started
            .as_ref()
            .map(|started| started.handle.clone())
            .ok_or_else(|| anyhow!("scheduler not started"))?;
        let proxy = module::run_proxy(config).await.map_err(|e| anyhow!(e))?;
        proxy
            .call(member, (handle,))
            .await
            .map_err(|error| anyhow!(error.wire_message()))
    }

    /// Read the due jobs and dispatch them; the count dispatched. Errors when
    /// the store cannot be read.
    pub(crate) async fn tick(&mut self, config: &Config) -> Result<usize> {
        self.binding.set_config(config);
        let dispatched: usize = self.call(config, members::TICK_SCHEDULER).await?;
        tracing::trace!(dispatched, "[cron:scheduler] TickScheduler");
        Ok(dispatched)
    }

    /// Wait for every dispatched job to finish.
    pub(crate) async fn drain(&mut self) {
        let config = self.binding.config();
        if self.started.is_none() {
            return;
        }
        if let Err(error) = self.call::<()>(&config, members::DRAIN_SCHEDULER).await {
            tracing::warn!(%error, "[cron:dispatch] a dispatched job failed");
        }
    }
}

impl Drop for JobDispatcher {
    fn drop(&mut self) {
        let Some(started) = self.started.take() else {
            return;
        };
        let config = self.binding.config();
        let context_pin = self.binding.pin_context();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Stop the module's scheduler (releasing its claims) before the
        // callback object it would call goes away. A failed or ambiguous stop
        // is retried while the registration remains owned by this task.
        runtime.spawn(async move {
            let Started { handle, host } = started;
            let _context_pin = context_pin;
            let _host_lifetime = host;
            let mut retry_delay = Duration::from_millis(100);
            loop {
                let stopped = match module::proxy(&config).await {
                    Ok(proxy) => proxy
                        .call::<()>(members::STOP_SCHEDULER, (handle.clone(),))
                        .await
                        .map_err(|error| error.wire_message()),
                    Err(error) => Err(error),
                };
                match stopped {
                    Ok(()) => break,
                    Err(error) => tracing::debug!(%error, "[cron:scheduler] retrying StopScheduler while retaining callback"),
                }
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
            }
        });
    }
}

/// Run manual ownership on the process-lived module runtime. A caller may
/// stop driving its runtime immediately after receiving the result (including
/// by entering a synchronous store bridge), while acknowledgement still uses
/// the shared module connection.
fn spawn_manual_owner(
    work: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<tokio::task::JoinHandle<()>> {
    #[cfg(feature = "modules")]
    {
        Ok(crate::modules::runtime::spawn_cleanup(work)?)
    }
    #[cfg(not(feature = "modules"))]
    {
        Ok(tokio::spawn(work))
    }
}

#[cfg(all(test, feature = "modules"))]
#[path = "dispatcher_tests.rs"]
mod tests;
