//! A library agent's scheduled reply reaches the host's `CronDelivery`.
//!
//! The agent makes a one-shot job through the `cron` tool in a chat turn on a
//! stateless runtime. When the scheduler runs it, the reply is handed to the
//! host with the agent's id and the thread the job was made in, not written
//! into a local thread.
//!
//! One runtime per process, so one test in this binary.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{offline_config, runtime, scripted_provider, stub_backend, tool_call_completion};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, CronDelivery, CronReply,
    HostTurnTools, InMemorySessionStores, JobOrigin, Runtime, ServiceSet, ToolScopeSpec, Workspace,
};

#[derive(Default)]
struct Recorder(Mutex<Vec<CronReply>>);

#[async_trait::async_trait]
impl CronDelivery for Recorder {
    async fn deliver(&self, reply: CronReply) -> Result<(), String> {
        self.0.lock().unwrap().push(reply);
        Ok(())
    }
}

/// A `cron add` call for a one-shot agent job a few seconds from now.
fn add_job(name: &str, prompt: &str) -> serde_json::Value {
    // A provider turn plus scheduler setup can take longer than two seconds,
    // especially under coverage. Keep the one-shot in the future until the
    // cron tool has committed it.
    let at = chrono::Utc::now() + chrono::Duration::seconds(10);
    let arguments = serde_json::json!({
        "action": "add", "job_type": "agent", "name": name, "prompt": prompt,
        "delivery": {"mode": "origin"},
        "schedule": {"kind": "at", "at": at.to_rfc3339()},
    });
    tool_call_completion("cron", &arguments.to_string())
}

#[test]
fn a_library_agents_scheduled_reply_is_handed_to_the_host() {
    let _ = env_logger::builder().is_test(true).try_init();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            // The chat turn schedules the job and acknowledges it; the job's
            // own run answers from the fallback.
            let provider = scripted_provider(
                vec![
                    add_job("stretch", "stretch reminder"),
                    common::chat_completion("Scheduled."),
                ],
                "Time to stretch!",
            )
            .await;
            let recorder = Arc::new(Recorder::default());
            let mut config = offline_config();
            config.reliability.scheduler_poll_secs = 5;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Stateless)
                .session_store(Arc::new(InMemorySessionStores::new()))
                .api_key("th_test_key")
                .backend_url(backend.uri())
                .cron_delivery(recorder.clone())
                .services(ServiceSet {
                    cron: true,
                    ..ServiceSet::none()
                })
                .build()
                .await
                .expect("runtime builds");
            let agent = runtime
                .agent(
                    AgentSpec::new("teeny")
                        .provider(common::route(&provider, "fixture"))
                        // Scheduling is the one effect this agent may take
                        // without a person approving it.
                        .access(Access::supervised().auto_approve(["cron"]))
                        .definition(
                            AgentDefinitionSpec::new()
                                .system_prompt("TEENY")
                                .tools(ToolScopeSpec::Named(vec!["cron".into(), "tool_search".into()])),
                        )
                        .tools(|_| HostTurnTools::advertised(Vec::new())),
                )
                .expect("agent instantiates");

            agent
                .turn("remind me to stretch")
                .session("t-stretch")
                // The host's conversation: what a job made in it returns to.
                .origin(AgentTurnOrigin::WebChat {
                    thread_id: "t-stretch".into(),
                    client_id: "teeny-host".into(),
                    request_id: None,
                })
                .send()
                .await
                .expect("the stretch turn runs");

            let mut delivered = Vec::new();
            for _ in 0..120 {
                delivered = recorder.0.lock().unwrap().clone();
                if !delivered.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            assert_eq!(delivered.len(), 1, "{delivered:?}");
            let reply = &delivered[0];
            assert_eq!(reply.agent_id.as_deref(), Some("teeny"));
            assert_eq!(reply.profile_id, None, "a library agent has no profile");
            assert_eq!(reply.thread_id.as_deref(), Some("t-stretch"));
            assert_eq!(reply.text, "Time to stretch!");
            assert!(
                matches!(&reply.origin, Some(JobOrigin::Web { thread_id, .. }) if thread_id == "t-stretch"),
                "{:?}",
                reply.origin
            );
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}
