//! The `tinyeventqueues` module linked into the test binary on a private bus,
//! with the host callbacks served by a scripted triage runner.
//!
//! Each [`Rig`] is its own broker and module host: the real module (its
//! `static-link` build) is attached with fast timers, the host's
//! `ai.tinyhumans.tinyeventqueues.Host` is served by an [`InboundHost`] whose
//! [`Deliverer`] triages with a [`ScriptedTriage`], and [`Rig::client`] is a
//! module client bound to the module's object, as `route::offer` takes it.
//! Nothing here talks to a model, a backend or the network.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinybus::broker::Broker;
use tinybus::module::{ModuleClient, ModuleHost};
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, ObjectPath};
use tinyeventqueues_bus::names;
use tinyeventqueues_bus::storage::SHIPPED_PIPELINE_JSON;
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, ServedStorage, StorageServer};
use tinystoragedrivers_testkit::MemoryBackend;

use super::deliver::Deliverer;
use super::host::InboundHost;
use super::pinned::{Pinned, PinnedAgent, PinnedRun};
use super::results::Results;
use crate::config::schema::{InboundConfig, InboundSourceConfig};
use crate::core::runtime::DispatchCtx;
use crate::triage::{
    TriageAction, TriageDecision, TriageOutcome, TriageResolutionPath, TriageRun, TriageRunner,
    TriggerEnvelope,
};

/// A decision of `action`.
pub(crate) fn decision(action: TriageAction) -> Step {
    Step::Answer(decision_outcome(action))
}

/// The outcome of a decision of `action`.
pub(crate) fn decision_outcome(action: TriageAction) -> TriageOutcome {
    TriageOutcome::Decision(TriageRun {
        decision: TriageDecision {
            action,
            target_agent: None,
            prompt: None,
            reason: "scripted".to_owned(),
        },
        used_local: false,
        latency_ms: 1,
        resolution_path: TriageResolutionPath::Cloud,
    })
}

/// One scripted answer.
#[derive(Debug, Clone)]
pub(crate) enum Step {
    /// Answer this outcome.
    Answer(TriageOutcome),
    /// Defer until `after_ms` after the triage that answers it.
    DeferFor(i64),
    /// Fail the triage with this message.
    Fail(&'static str),
    /// Never answer (until the caller's timeout).
    Hang,
}

impl From<TriageOutcome> for Step {
    fn from(outcome: TriageOutcome) -> Self {
        Self::Answer(outcome)
    }
}

/// A deferral of `after_ms`, counted from the triage that answers it.
pub(crate) fn deferred(after_ms: i64) -> Step {
    Step::DeferFor(after_ms)
}

/// A triage runner that answers from a script and records what it did.
#[derive(Debug, Default)]
pub(crate) struct ScriptedTriage {
    script: Mutex<VecDeque<Step>>,
    runs: AtomicUsize,
    applied: Mutex<Vec<String>>,
    failing_applies: AtomicUsize,
    apply_delay: Mutex<Duration>,
}

impl ScriptedTriage {
    /// Answer `steps` in order, then acknowledge.
    pub(crate) fn new<S: Into<Step>>(steps: impl IntoIterator<Item = S>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(steps.into_iter().map(Into::into).collect()),
            ..Self::default()
        })
    }

    /// Fail the next `n` applies.
    pub(crate) fn fail_next_applies(&self, n: usize) {
        self.failing_applies.store(n, Ordering::SeqCst);
    }

    /// Take `delay` over every apply.
    pub(crate) fn slow_applies(&self, delay: Duration) {
        *self.apply_delay.lock().unwrap() = delay;
    }

    /// How many triage runs happened.
    pub(crate) fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }

    /// The labels of the envelopes whose decisions were applied.
    pub(crate) fn applied(&self) -> Vec<String> {
        self.applied.lock().unwrap().clone()
    }

    /// Wait until `n` decisions were applied, or `within` passed.
    pub(crate) async fn until_applied(&self, n: usize, within: Duration) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let applied = self.applied();
            if applied.len() >= n || tokio::time::Instant::now() >= deadline {
                return applied;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[async_trait]
impl TriageRunner for ScriptedTriage {
    async fn run(
        &self,
        _ctx: &DispatchCtx,
        _envelope: &TriggerEnvelope,
    ) -> anyhow::Result<TriageOutcome> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let next = self.script.lock().unwrap().pop_front();
        match next {
            Some(Step::Fail(message)) => return Err(anyhow::anyhow!(message)),
            Some(Step::Hang) => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                return Err(anyhow::anyhow!("hung"));
            }
            _ => {}
        }
        Ok(match next {
            Some(Step::Answer(outcome)) => outcome,
            Some(Step::DeferFor(after_ms)) => TriageOutcome::Deferred {
                defer_until_ms: chrono::Utc::now().timestamp_millis() + after_ms,
                reason: "scripted deferral".to_owned(),
            },
            Some(Step::Fail(_) | Step::Hang) | None => decision_outcome(TriageAction::Acknowledge),
        })
    }

    async fn apply(
        &self,
        _ctx: &DispatchCtx,
        _run: TriageRun,
        envelope: &TriggerEnvelope,
    ) -> anyhow::Result<()> {
        let delay = *self.apply_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let failing = self
            .failing_applies
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failing {
            return Err(anyhow::anyhow!("scripted apply failure"));
        }
        self.applied
            .lock()
            .unwrap()
            .push(envelope.display_label.clone());
        Ok(())
    }
}

/// One scripted pinned-agent answer.
#[derive(Debug, Clone)]
pub(crate) enum Reply {
    /// Answer this text.
    Text(&'static str),
    /// Fail with this message.
    Fail(&'static str),
    /// Answer this text after this long.
    Slow(Duration, &'static str),
    /// Fail with this message after this long.
    SlowFail(Duration, &'static str),
}

/// A pinned agent that answers from a script (then `"ok"`) and records its
/// runs.
#[derive(Debug, Default)]
pub(crate) struct ScriptedAgent {
    script: Mutex<VecDeque<Reply>>,
    runs: Mutex<Vec<PinnedRun>>,
}

impl ScriptedAgent {
    pub(crate) fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(replies.into_iter().collect()),
            ..Self::default()
        })
    }

    /// The runs so far.
    pub(crate) fn runs(&self) -> Vec<PinnedRun> {
        self.runs.lock().unwrap().clone()
    }
}

#[async_trait]
impl PinnedAgent for ScriptedAgent {
    async fn run(&self, _ctx: &DispatchCtx, run: &PinnedRun) -> anyhow::Result<String> {
        self.runs.lock().unwrap().push(run.clone());
        let next = self.script.lock().unwrap().pop_front();
        match next {
            Some(Reply::Text(text)) => Ok(text.to_owned()),
            Some(Reply::Fail(message)) => Err(anyhow::anyhow!(message)),
            Some(Reply::Slow(delay, text)) => {
                tokio::time::sleep(delay).await;
                Ok(text.to_owned())
            }
            Some(Reply::SlowFail(delay, message)) => {
                tokio::time::sleep(delay).await;
                Err(anyhow::anyhow!(message))
            }
            None => Ok("ok".to_owned()),
        }
    }
}

/// A results store in fresh memory storage.
pub(crate) fn memory_results() -> Results {
    let storage =
        tinystoragedrivers_testkit::memory_storage(&crate::storage::Scope::new("local").unwrap());
    Results::over(&storage)
}

/// `[inbound]` with `x:mentions` pinned to `teeny-mentions`, flushing after
/// `debounce_ms` and carrying at most `max_batch` events a turn.
pub(crate) fn mentions_inbound(debounce_ms: u64, max_batch: u32) -> InboundConfig {
    let mut inbound = InboundConfig::default();
    inbound.sources.insert(
        "x:mentions".to_owned(),
        InboundSourceConfig {
            agent_id: "teeny-mentions".to_owned(),
            debounce_ms,
            max_wait_ms: debounce_ms * 20,
            max_batch,
            ..InboundSourceConfig::default()
        },
    );
    inbound
}

/// Wait until `results` holds a finished result for `turn_id`, or `within`
/// passed.
pub(crate) async fn until_finished(
    results: &Results,
    turn_id: &str,
    within: Duration,
) -> Option<super::results::Stored> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let found = results.get(turn_id).unwrap();
        let finished = found
            .as_ref()
            .is_some_and(|stored| stored.result.status != super::results::ResultStatus::Running);
        if finished || tokio::time::Instant::now() >= deadline {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The shipped pipeline with a short debounce, so a test does not wait the
/// production 1.5 s per flush.
fn fast_pipeline() -> Value {
    let mut pipeline: Value = serde_json::from_str(SHIPPED_PIPELINE_JSON).unwrap();
    pipeline["coalesce"]["debounce_ms"] = json!(10);
    pipeline["coalesce"]["max_wait_ms"] = json!(50);
    pipeline
}

/// One private bus with the module attached and the host callbacks served.
pub(crate) struct Rig {
    /// Module client bound to the module's object.
    pub(crate) client: ModuleClient,
    /// The triage the host callbacks run.
    pub(crate) triage: Arc<ScriptedTriage>,
    _host: Connection,
    _storage: ServedStorage,
    _modules: ModuleHost,
}

impl Rig {
    /// Attach the module and serve the host with `triage`.
    pub(crate) async fn new(triage: Arc<ScriptedTriage>) -> Self {
        Self::build(triage, None).await
    }

    /// Attach the module with `inbound`'s sources in its pipeline, and serve
    /// the host with `pinned` running them.
    pub(crate) async fn pinned(inbound: InboundConfig, pinned: Pinned) -> Self {
        Self::build(
            ScriptedTriage::new(Vec::<Step>::new()),
            Some((inbound, pinned)),
        )
        .await
    }

    async fn build(triage: Arc<ScriptedTriage>, pinned: Option<(InboundConfig, Pinned)>) -> Self {
        let bus = MemoryBus::new();
        let broker = Broker::new();
        broker.clone().spawn(bus.clone());
        let host = Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap();
        let deliverer = Deliverer::new(
            triage.clone() as Arc<dyn TriageRunner>,
            DispatchCtx::host_default(),
            Duration::from_secs(5),
        )
        .with_max_attempts(|_| Some(5));
        let mut pipeline = fast_pipeline();
        let deliverer = match pinned {
            Some((inbound, pinned)) => {
                pipeline = super::settings::with_sources(pipeline, &inbound);
                deliverer.with_inbound(inbound).with_pinned(pinned)
            }
            None => deliverer,
        };
        host.serve_at(
            ObjectPath::new(names::host::OBJECT_PATH).unwrap(),
            InboundHost::new(Arc::new(deliverer)),
        )
        .await
        .unwrap();
        host.request_name(names::host::INTERFACE).await.unwrap();
        // The module keeps its queues in host storage and refuses to set up
        // without it, so the rig serves the Storage service from memory.
        let backend = Arc::new(MemoryBackend::new());
        let storage = tinystoragedrivers_busproxy::serve(
            &host,
            Arc::new(PerCallerEndpoint::new(move |_caller| {
                let storage = backend.for_scope(&crate::storage::Scope::local()).ok()?;
                Some(StorageServer::new(storage))
            })),
        )
        .await
        .unwrap();

        let modules = ModuleHost::new(broker);
        modules
            .attach_linked_with_config(
                tinyeventqueues::linked_module().unwrap(),
                json!({
                    "storage_callback": tinystoragedrivers_bus::STORAGE_INTERFACE,
                    "pipeline": pipeline,
                    "timer": { "min_interval_ms": 5, "max_interval_ms": 25 },
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let names = host.list_names().await.unwrap();
                if names.iter().any(|name| name.as_str() == names::INTERFACE) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the linked tinyeventqueues module claimed its name");
        let proxy = host
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
            .unwrap();
        Self {
            client: crate::modules::ops::fixture_client(proxy),
            triage,
            _host: host,
            _storage: storage,
            _modules: modules,
        }
    }
}

/// A config with the inbound event queues active.
pub(crate) fn active_config() -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.modules.enabled = true;
    config.components.eventqueues = true;
    config.inbound.enabled = true;
    config
}
