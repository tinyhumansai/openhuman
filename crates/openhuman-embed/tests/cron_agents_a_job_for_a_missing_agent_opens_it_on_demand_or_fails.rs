//! A due job whose agent is not live runs as the agent the host's
//! `on_missing_agent` opener returns, and fails (never runs as another agent)
//! when the opener declines or there is none.

mod common;

use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use common::{chat_completion, offline_config, runtime, stub_backend};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, JobSchedule, JobSpec, Provider, Runtime, ToolScopeSpec,
    Workspace,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HOURLY: JobSchedule = JobSchedule::Every { ms: 3_600_000 };

#[test]
fn a_job_for_a_missing_agent_opens_it_on_demand_or_fails() {
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(chat_completion("opened-and-done")),
                )
                .mount(&provider)
                .await;
            let provider_url = format!("{}/v1", provider.uri());

            let handle: Arc<OnceLock<Weak<Runtime>>> = Arc::new(OnceLock::new());
            let opener_handle = Arc::clone(&handle);
            let runtime = Arc::new(
                Runtime::builder()
                    .config(offline_config())
                    .workspace(Workspace::Ephemeral)
                    .backend_url(backend.uri())
                    .on_missing_agent(move |id| {
                        let handle = Arc::clone(&opener_handle);
                        let provider_url = provider_url.clone();
                        async move {
                            if id != "sleeper" {
                                return None;
                            }
                            let runtime = handle.get()?.upgrade()?;
                            runtime
                                .agent(
                                    AgentSpec::new("sleeper")
                                        .provider(
                                            Provider::openai_compatible(provider_url, "sk-fixture")
                                                .model("fixture"),
                                        )
                                        .definition(
                                            AgentDefinitionSpec::new()
                                                .system_prompt("SLEEPER_PROMPT")
                                                .tools(ToolScopeSpec::Named(Vec::new())),
                                        ),
                                )
                                .ok()
                        }
                    })
                    .build()
                    .await
                    .expect("runtime builds"),
            );
            handle.set(Arc::downgrade(&runtime)).expect("set once");
            let cron = runtime.cron();

            assert!(runtime.agent_ids().is_empty(), "no agent is live");
            cron.upsert(JobSpec::agent("sleeper-hourly", "sleeper", "wake", HOURLY).retries(0))
                .expect("job created");
            let run = cron.run_now("sleeper-hourly").await.expect("job runs");
            assert!(run.success, "{}", run.output);
            assert!(run.output.contains("opened-and-done"), "{}", run.output);
            let requests = provider.received_requests().await.expect("recorded");
            assert!(
                requests
                    .iter()
                    .any(|r| String::from_utf8_lossy(&r.body).contains("SLEEPER_PROMPT")),
                "the turn ran with the opened agent's own prompt"
            );

            // The host declines this one: the job fails, nothing else runs it.
            let before = provider.received_requests().await.expect("recorded").len();
            cron.upsert(JobSpec::agent("ghost-hourly", "ghost", "wake", HOURLY).retries(0))
                .expect("job created");
            let run = cron.run_now("ghost-hourly").await.expect("job runs");
            assert!(!run.success, "{}", run.output);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                provider.received_requests().await.expect("recorded").len(),
                before,
                "a missing agent's job must not run as the canonical orchestrator"
            );
        })
        .await
        .expect("scenario completes");
    });
}
