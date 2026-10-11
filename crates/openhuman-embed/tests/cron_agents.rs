//! OpenHuman's own cron driving runtime agents.
//!
//! A scheduled job that targets a runtime agent runs *as that agent*: its
//! system prompt, its host tools and its context, under the
//! `TrustedAutomation { Cron }` origin. A system job's in-process handler
//! decides the run's recorded result. And a runtime that asks for background
//! services runs the scheduler, then stops it when it is dropped.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{chat_completion, chat_requests, offline_config, runtime, stub_backend};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, CronError, HostTurnTools, JobSchedule,
    JobSpec, JobTarget, Provider, Runtime, RuntimeError, ServiceSet, Tool, ToolScopeSpec,
    TrustedAutomationSource, Workspace,
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
            .push(openhuman_core::agent::turn_origin::current());
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

#[test]
fn a_scheduled_job_runs_a_runtime_agent_with_its_host_tool_as_cron() {
    let _lock = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = provider_calling_ping().await;
            let runtime = Runtime::builder()
                .config(offline_config())
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
                            .model("fixture"),
                        )
                        .definition(
                            AgentDefinitionSpec::new()
                                .system_prompt("TEENY_CRON_PROMPT")
                                .tools(ToolScopeSpec::Named(vec!["teeny_ping".into()])),
                        )
                        .tools(move |_| {
                            HostTurnTools::advertised(vec![Box::new(Ping {
                                seen: Arc::clone(&belt_seen),
                            })])
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

            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}

#[test]
fn background_services_drive_the_scheduler_and_stop_with_the_runtime() {
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
                .build()
                .await
                .expect("runtime builds");
            wait_for(
                "the scheduler to start",
                openhuman_core::cron::scheduler::is_running,
            )
            .await;

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

            assert!(matches!(
                Runtime::builder().build().await,
                Err(RuntimeError::AlreadyRunning)
            ));

            // Explicit control.
            runtime.stop_services();
            wait_for("the scheduler to stop", || {
                !openhuman_core::cron::scheduler::is_running()
            })
            .await;
            runtime.start_services().await;
            wait_for(
                "the scheduler to restart",
                openhuman_core::cron::scheduler::is_running,
            )
            .await;

            drop(runtime);
            wait_for("the scheduler to stop with the runtime", || {
                !openhuman_core::cron::scheduler::is_running()
            })
            .await;
            let second = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .expect("a runtime builds once the first is gone");
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
#[test]
fn deferred_services_wait_for_the_host_so_a_due_job_finds_its_handler() {
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
                .expect("runtime builds");

            // Stored and due before any handler exists.
            runtime
                .cron()
                .upsert(JobSpec::system(
                    "deferred-tick",
                    "deferred-tick",
                    JobSchedule::Every { ms: 1_000 },
                ))
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(
                !openhuman_core::cron::scheduler::is_running(),
                "a deferred build must not start the scheduler"
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
            wait_for(
                "the scheduler to start",
                openhuman_core::cron::scheduler::is_running,
            )
            .await;
            wait_for("the handler to run the due job", || {
                *fired.lock().unwrap() > 0
            })
            .await;

            drop(runtime);
            wait_for("the scheduler to stop with the runtime", || {
                !openhuman_core::cron::scheduler::is_running()
            })
            .await;
        })
        .await
        .expect("library host task did not panic");
    });
}
