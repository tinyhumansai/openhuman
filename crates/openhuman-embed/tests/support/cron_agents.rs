//! OpenHuman's own cron driving runtime agents.
//!
//! A scheduled job that targets a runtime agent runs *as that agent*: its
//! system prompt, its host tools and its context, under the
//! `TrustedAutomation { Cron }` origin. A system job's in-process handler
//! decides the run's recorded result. And a runtime that asks for background
//! services runs the scheduler, then stops it when it is dropped.

#![allow(dead_code)] // Each test target includes the shared scenarios but runs one case.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{chat_completion, chat_requests, offline_config, runtime, stub_backend};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, CronError, HostTurnTools, JobSchedule,
    JobSpec, JobTarget, Provider, ReasoningEffort, Runtime, RuntimeError, RuntimeEventKind,
    RuntimeEvents, ServiceSet, SessionStoreProvider, Tool, ToolScopeSpec, TrustedAutomationSource,
    Workspace,
};
use tinyagents_session::port::{AgentStores, InMemorySessionStores};
use tinyagents_session::transcript::{
    SessionRef, ThreadTranscripts, TranscriptHistory, TranscriptLocator, TranscriptMessage,
    TranscriptMeta, TranscriptPartial, TranscriptRead,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// One runtime per process; the tests in this file take turns.
static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

const HOURLY: JobSchedule = JobSchedule::Every { ms: 3_600_000 };

/// A host tool that records the authority each call ran under.
struct Ping {
    seen: Arc<Mutex<Vec<Option<AgentTurnOrigin>>>>,
}

#[async_trait::async_trait]
impl Tool for Ping {
    fn name(&self) -> &str {
        "teeny_ping"
    }
    fn description(&self) -> &str {
        "Tell the host the scheduled check-in happened"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        self.seen
            .lock()
            .unwrap()
            .push(openhuman_core::core::runtime::turn_origin::current());
        Ok(tinytools::ToolResult::success("pinged"))
    }
}

/// A provider that asks for `teeny_ping` once and then answers.
async fn provider_calling_ping() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(common::tool_call_completion("teeny_ping", "{}")),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("cron-done")))
        .mount(&server)
        .await;
    server
}

async fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..240 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn wait_for_services_changed(events: &mut RuntimeEvents, running: bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = events.recv().await.expect("runtime event stream");
            if matches!(event.kind, RuntimeEventKind::ServicesChanged { running: value } if value == running)
            {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for services running={running}"));
}

pub fn a_scheduled_job_runs_a_runtime_agent_with_its_host_tool_as_cron() {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = provider_calling_ping().await;
            let config = offline_config();
            let runtime = Runtime::builder()
                .config(config.clone())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .expect("runtime builds");

            let seen = Arc::new(Mutex::new(Vec::new()));
            let belt_seen = Arc::clone(&seen);
            let agent = runtime
                .agent(
                    AgentSpec::new("teeny")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", provider.uri()),
                                "sk-fixture",
                            )
                            .model("fixture")
                            .provider_options(serde_json::json!({"provider": {"sort": "latency"}}))
                            .reasoning_effort(ReasoningEffort::High),
                        )
                        .definition(
                            AgentDefinitionSpec::new()
                                .system_prompt("TEENY_CRON_PROMPT")
                                .tools(ToolScopeSpec::Named(vec!["teeny_ping".into()])),
                        )
                        .tools(move |_| {
                            let seen = Arc::clone(&belt_seen);
                            Box::pin(async move {
                                HostTurnTools::advertised(vec![Box::new(Ping { seen })])
                            })
                        }),
                )
                .expect("agent instantiates");

            let cron = runtime.cron();

            // Upsert is idempotent by name.
            let spec = JobSpec::agent("teeny-hourly", "teeny", "say hi", HOURLY)
                .retries(0)
                .single_flight(true);
            let first = cron.upsert(spec.clone()).expect("job created");
            let again = cron.upsert(spec).expect("job upserted");
            assert_eq!(first.id, again.id);
            assert_eq!(again.retries, Some(0));
            assert!(again.single_flight);
            assert_eq!(cron.list().unwrap().len(), 1);
            let changed = cron
                .upsert(JobSpec::agent("teeny-hourly", "teeny", "check in", HOURLY))
                .expect("job updated");
            assert_eq!(changed.id, first.id, "same job, updated in place");
            assert_eq!(
                changed.target,
                JobTarget::Agent {
                    agent_id: "teeny".into(),
                    prompt: "check in".into()
                }
            );
            assert_eq!(changed.retries, None);
            assert!(!changed.single_flight);
            runtime
                .core_runtime()
                .invoke(
                    "openhuman.cron_update",
                    serde_json::json!({
                        "job_id": changed.id,
                        "patch": {"model": "cron-override"}
                    }),
                )
                .await
                .unwrap_or_else(|error| panic!("job model override was not stored: {error}"));

            // The run goes through the cron scheduler's agent path.
            let run = cron.run_now("teeny-hourly").await.expect("job runs");
            assert!(run.success, "{}", run.output);
            assert!(run.output.contains("cron-done"), "{}", run.output);

            let origins = seen.lock().unwrap().clone();
            assert_eq!(origins.len(), 1, "the host tool was called once");
            assert!(
                matches!(
                    &origins[0],
                    Some(AgentTurnOrigin::TrustedAutomation {
                        job_id,
                        source: TrustedAutomationSource::Cron,
                    }) if *job_id == first.id
                ),
                "{origins:?}"
            );
            let requests = chat_requests(&provider).await;
            let first_request = String::from_utf8_lossy(&requests[0].body).to_string();
            assert!(first_request.contains("TEENY_CRON_PROMPT"));
            let first_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(
                first_body["provider"],
                serde_json::json!({"sort": "latency"})
            );
            assert_eq!(first_body["reasoning_effort"], "high");
            assert!(common::tool_names(&requests[0]).contains(&"teeny_ping".to_string()));
            assert_eq!(cron.runs("teeny-hourly", 10).unwrap()[0].status, "ok");

            // A system job's handler result is the run's result.
            let fired = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&fired);
            runtime
                .on_system_job("teeny-digest", move |ctx| {
                    sink.lock().unwrap().push(ctx.name.clone());
                    async { Err("digest failed".to_string()) }
                })
                .expect("handler registers");
            cron.upsert(JobSpec::system("teeny-digest", "teeny-digest", HOURLY).retries(0))
                .expect("system job created");
            let run = cron.run_now("teeny-digest").await.expect("system job runs");
            assert!(!run.success);
            assert!(run.output.contains("digest failed"), "{}", run.output);
            assert_eq!(*fired.lock().unwrap(), vec!["teeny-digest".to_string()]);
            let digest = cron
                .list()
                .unwrap()
                .into_iter()
                .find(|job| job.name == "teeny-digest")
                .expect("listed");
            assert_eq!(digest.last_status.as_deref(), Some("error"));

            assert!(cron.remove("teeny-digest").unwrap());
            assert!(!cron.remove("teeny-digest").unwrap());
            assert!(matches!(
                cron.run_now("teeny-digest").await,
                Err(CronError::NotFound(_))
            ));

            runtime.remove_agent("teeny").await.unwrap();
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}

pub fn background_services_drive_the_scheduler_and_stop_with_the_runtime() {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let mut config = offline_config();
            config.reliability.scheduler_poll_secs = 5;
            let temp = tempfile::tempdir().expect("workspace");
            let workspace = Workspace::dir(temp.path().join("state"));
            let runtime = Runtime::builder()
                .config(config.clone())
                .workspace(workspace.clone())
                .backend_url(backend.uri())
                .services(ServiceSet {
                    cron: true,
                    ..ServiceSet::none()
                })
                .build()
                .await
                .unwrap_or_else(|error| {
                    eprintln!("background scheduler runtime build failed: {error:?}");
                    panic!("runtime builds");
                });
            let mut events = runtime.events();

            let fired = Arc::new(Mutex::new(0usize));
            let sink = Arc::clone(&fired);
            runtime
                .on_system_job("svc-tick", move |_ctx| {
                    *sink.lock().unwrap() += 1;
                    async { Ok(()) }
                })
                .unwrap();
            runtime
                .cron()
                .upsert(JobSpec::system(
                    "svc-tick",
                    "svc-tick",
                    JobSchedule::Every { ms: 1_000 },
                ))
                .unwrap();
            wait_for("the scheduler to fire the job", || {
                *fired.lock().unwrap() > 0
            })
            .await;
            wait_for("the first scheduled run to finish", || {
                runtime
                    .cron()
                    .runs("svc-tick", 10)
                    .is_ok_and(|runs| runs.first().is_some_and(|run| run.status == "ok"))
            })
            .await;
            let runs_after_first = runtime.cron().runs("svc-tick", 10).unwrap().len();
            assert!(runs_after_first > 0, "the scheduler records its callback");

            assert!(matches!(
                Runtime::builder().build().await,
                Err(RuntimeError::AlreadyRunning)
            ));

            // Explicit control.
            runtime.stop_services();
            wait_for_services_changed(&mut events, false).await;
            // The host scheduler clamps the configured poll cadence to at least five
            // seconds, so observe beyond one full effective poll interval.
            tokio::time::sleep(Duration::from_millis(6_200)).await;
            assert_eq!(
                runtime.cron().runs("svc-tick", 10).unwrap().len(),
                runs_after_first,
                "stopped services must not run another due job"
            );
            runtime.start_services().await;
            wait_for_services_changed(&mut events, true).await;
            wait_for("the scheduler to complete a run after restart", || {
                runtime.cron().runs("svc-tick", 10).is_ok_and(|runs| {
                    runs.len() > runs_after_first
                        && runs.first().is_some_and(|run| run.status == "ok")
                })
            })
            .await;
            let runs_after_restart = runtime.cron().runs("svc-tick", 10).unwrap().len();

            drop(runtime);
            tokio::time::sleep(Duration::from_millis(6_200)).await;
            let second = Runtime::builder()
                .config(config)
                .workspace(workspace)
                .backend_url(backend.uri())
                .build()
                .await
                .expect("a runtime builds once the first is gone");
            assert_eq!(
                second.cron().runs("svc-tick", 10).unwrap().len(),
                runs_after_restart,
                "dropping the runtime must stop its scheduler"
            );
            drop(second);
        })
        .await
        .expect("library host task did not panic");
    });
}

/// A deferred runtime leaves its scheduler stopped until the host starts it,
/// so a system job already due at boot finds its handler in place: here the
/// job is stored and due before the handler exists, and the first run the
/// scheduler records is the handler's.
pub fn deferred_services_wait_for_the_host_so_a_due_job_finds_its_handler() {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let mut config = offline_config();
            config.reliability.scheduler_poll_secs = 5;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .services(ServiceSet {
                    cron: true,
                    ..ServiceSet::none()
                })
                .defer_services()
                .build()
                .await
                .unwrap_or_else(|error| {
                    eprintln!("deferred scheduler runtime build failed: {error:?}");
                    panic!("runtime builds");
                });
            let mut events = runtime.events();

            // Stored and due before any handler exists.
            runtime
                .cron()
                .upsert(JobSpec::system(
                    "deferred-tick",
                    "deferred-tick",
                    JobSchedule::Every { ms: 1_000 },
                ))
                .unwrap();
            // The scheduler clamps host polling to five seconds; wait beyond
            // that interval so an incorrectly-started scheduler would run.
            tokio::time::sleep(Duration::from_millis(6_200)).await;
            assert!(
                runtime.cron().runs("deferred-tick", 10).unwrap().is_empty(),
                "a deferred build must not run the due job"
            );

            let fired = Arc::new(Mutex::new(0usize));
            let sink = Arc::clone(&fired);
            runtime
                .on_system_job("deferred-tick", move |_ctx| {
                    *sink.lock().unwrap() += 1;
                    async { Ok(()) }
                })
                .unwrap();
            runtime.start_services().await;
            wait_for_services_changed(&mut events, true).await;
            wait_for("the handler to run the due job", || {
                *fired.lock().unwrap() > 0
            })
            .await;

            drop(runtime);
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(5), events.recv()).await,
                Ok(Err(openhuman_embed::EventStreamError::Closed))
            ));
        })
        .await
        .expect("library host task did not panic");
    });
}

type Bound = Arc<Mutex<Vec<(String, Arc<dyn TranscriptHistory>)>>>;

/// In-memory session stores that remember every transcript stem a turn binds.
#[derive(Default)]
struct RecordingStores {
    inner: InMemorySessionStores,
    bound: Bound,
}

impl SessionStoreProvider for RecordingStores {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        let mut stores = self.inner.for_agent(agent_id);
        stores.transcripts = Arc::new(RecordingLocator {
            inner: stores.transcripts,
            bound: Arc::clone(&self.bound),
        });
        stores
    }

    fn recover(&self) -> anyhow::Result<()> {
        self.inner.recover()
    }

    fn destination_key(&self) -> Option<String> {
        self.inner.destination_key()
    }
}

struct RecordingLocator {
    inner: Arc<dyn TranscriptLocator>,
    bound: Bound,
}

impl TranscriptLocator for RecordingLocator {
    fn destination_key(&self) -> Option<String> {
        self.inner.destination_key()
    }

    fn latest_for_agent(&self, agent_name: &str) -> Option<Arc<dyn TranscriptRead>> {
        self.inner.latest_for_agent(agent_name)
    }

    fn root_for_thread(&self, thread_id: &str) -> Option<Arc<dyn TranscriptRead>> {
        self.inner.root_for_thread(thread_id)
    }

    fn root_for_thread_scoped(
        &self,
        thread_id: &str,
        agent_id: Option<&str>,
    ) -> Option<Arc<dyn TranscriptRead>> {
        self.inner.root_for_thread_scoped(thread_id, agent_id)
    }

    fn thread_transcripts(
        &self,
        thread_id: &str,
        agent_id: Option<&str>,
    ) -> Option<ThreadTranscripts> {
        self.inner.thread_transcripts(thread_id, agent_id)
    }

    fn open_stem(
        &self,
        stem: &str,
        seed: TranscriptMeta,
    ) -> anyhow::Result<Arc<dyn TranscriptHistory>> {
        let handle = self.inner.open_stem(stem, seed)?;
        self.bound
            .lock()
            .unwrap()
            .push((stem.to_string(), Arc::clone(&handle)));
        Ok(handle)
    }

    fn append_interrupted_partial(
        &self,
        thread_id: &str,
        agent_id: Option<&str>,
        partial: &TranscriptPartial,
        request_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        self.inner
            .append_interrupted_partial(thread_id, agent_id, partial, request_id)
    }

    fn begin_generation(
        &self,
        session: &SessionRef,
        seed: TranscriptMeta,
    ) -> anyhow::Result<(SessionRef, Arc<dyn TranscriptHistory>)> {
        self.inner.begin_generation(session, seed)
    }

    fn begin_generation_from_baseline(
        &self,
        session: &SessionRef,
        seed: TranscriptMeta,
        baseline: &[TranscriptMessage],
    ) -> anyhow::Result<(SessionRef, Arc<dyn TranscriptHistory>)> {
        self.inner
            .begin_generation_from_baseline(session, seed, baseline)
    }
}

/// Two jobs that target one agent can run at the same time: each run keeps
/// its own transcript, so neither turn lands in, or overwrites, the other's.
pub fn overlapping_runs_of_one_agent_keep_separate_transcripts() {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(chat_completion("overlap-done"))
                        .set_delay(Duration::from_millis(300)),
                )
                .mount(&provider)
                .await;
            let store = Arc::new(RecordingStores::default());
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Stateless)
                .session_store(store.clone())
                .backend_url(backend.uri())
                .build()
                .await
                .expect("runtime builds");
            let agent = runtime
                .agent(
                    AgentSpec::new("overlap")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", provider.uri()),
                                "sk-fixture",
                            )
                            .model("fixture"),
                        )
                        .definition(AgentDefinitionSpec::new().system_prompt("OVERLAP_PROMPT")),
                )
                .expect("agent instantiates");

            let cron = runtime.cron();
            let jobs = [
                ("overlap-a", "check in alpha"),
                ("overlap-b", "check in bravo"),
            ];
            for (name, prompt) in jobs {
                cron.upsert(JobSpec::agent(name, "overlap", prompt, HOURLY).retries(0))
                    .expect("job created");
            }

            let (a, b) = tokio::join!(cron.run_now("overlap-a"), cron.run_now("overlap-b"));
            for run in [a.expect("job a runs"), b.expect("job b runs")] {
                assert!(run.success, "{}", run.output);
                assert!(run.output.contains("overlap-done"), "{}", run.output);
            }
            for (name, _) in jobs {
                assert_eq!(cron.runs(name, 1).unwrap()[0].status, "ok", "{name}");
            }

            let mut written: Vec<(String, String)> = Vec::new();
            for (stem, handle) in store.bound.lock().unwrap().iter() {
                if written.iter().any(|(known, _)| known == stem) {
                    continue;
                }
                if let Some(session) = handle.read_session().expect("readable") {
                    let users: Vec<String> = session
                        .messages
                        .iter()
                        .filter(|row| row.role == "user")
                        .map(|row| row.content.clone())
                        .collect();
                    written.push((stem.clone(), users.join("\n")));
                }
            }
            assert_eq!(written.len(), 2, "one transcript per run: {written:?}");
            for (_, prompt) in jobs {
                let holding: Vec<_> = written
                    .iter()
                    .filter(|(_, users)| users.contains(prompt))
                    .collect();
                assert_eq!(holding.len(), 1, "{prompt:?} in {written:?}");
                let (_, users) = holding[0];
                assert_eq!(
                    users.matches("check in").count(),
                    1,
                    "{prompt:?} shares its transcript: {users}"
                );
            }

            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}
