//! A Telegram bot answered by a runtime agent, end to end: a mocked Bot API
//! delivers a message, the agent bound to the bot answers it with its own
//! prompt and host tools under the `ExternalChannel` origin, its write tool is
//! refused at once, and the reply is posted back to the chat.

// The agent's turn persists its transcript in the ephemeral workspace's
// default store, which is SQLite: without that driver the turn cannot run.
#![cfg(all(feature = "channels", feature = "storage-sqlite"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{chat_completion, chat_requests, offline_config, runtime, stub_backend};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, ChannelError, HostTurnTools, Provider,
    Runtime, TelegramChannelSpec, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use tinytools::{PermissionLevel, ToolResult};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "123456:TEST";

type Origins = Arc<Mutex<Vec<Option<AgentTurnOrigin>>>>;

struct Probe {
    name: &'static str,
    level: PermissionLevel,
    runs: Arc<AtomicUsize>,
    origins: Origins,
}

#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "a host probe"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.origins
            .lock()
            .unwrap()
            .push(openhuman_core::core::runtime::CoreContext::current_turn_origin());
        Ok(ToolResult::success(format!("{} ran", self.name)))
    }
}

/// A Bot API that delivers one message from `alice` and accepts every send.
async fn telegram() -> MockServer {
    let server = MockServer::start().await;
    let updates = format!("/bot{TOKEN}/getUpdates");
    Mock::given(method("POST"))
        .and(path(updates.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": [{
                "update_id": 1,
                "message": {
                    "message_id": 7,
                    "date": 0,
                    "text": "hello teeny",
                    "from": { "id": 99, "is_bot": false, "username": "alice" },
                    "chat": { "id": 4242, "type": "private" }
                }
            }]
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(updates))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "ok": true, "result": [] }))
                .set_delay(Duration::from_millis(200)),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": { "message_id": 8, "id": 1, "is_bot": true, "username": "teeny_bot" }
        })))
        .with_priority(10)
        .mount(&server)
        .await;
    server
}

fn tool_call(tool: &str) -> Value {
    common::tool_call_completion(tool, "{}")
}

/// A provider that calls the read tool, then the write tool, then answers.
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    for (priority, body) in [(1, tool_call("teeny_read")), (2, tool_call("teeny_write"))] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .with_priority(priority)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("teeny says hi")))
        .with_priority(3)
        .mount(&server)
        .await;
    server
}

async fn sent_texts(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/sendMessage"))
        .filter_map(|r| {
            serde_json::from_slice::<Value>(&r.body)
                .ok()
                .and_then(|body| body["text"].as_str().map(str::to_string))
        })
        .collect()
}

#[test]
fn a_telegram_message_is_answered_by_the_bound_runtime_agent() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let bot_api = telegram().await;
            let provider = provider().await;
            // `start_channels` resolves the managed provider up front, and the
            // core carries none of its own; supply it as the TinyHumans layer
            // would, pointed at the mock backend.
            common::PointedInference::install(&format!("{}/openai/v1", backend.uri()));
            // The Bot API base is read when the channel is built.
            std::env::set_var("OPENHUMAN_TELEGRAM_BOT_API_BASE", bot_api.uri());

            let mut config = offline_config();
            if let Some(artifact) = openhuman_core::modules::local_artifact("tinychannels") {
                config.modules.overrides = vec![
                    openhuman_core::config::schema::ModuleOverride::with_path(
                        "tinychannels",
                        artifact.to_string_lossy(),
                    ),
                ];
            }
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .api_key("sk-managed-fixture")
                .build()
                .await
                .expect("runtime builds");

            let (reads, writes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            let origins: Origins = Arc::new(Mutex::new(Vec::new()));
            let (belt_reads, belt_writes, belt_origins) = (
                Arc::clone(&reads),
                Arc::clone(&writes),
                Arc::clone(&origins),
            );
            let agent = runtime
                .agent(
                    AgentSpec::new("teeny-chat")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", provider.uri()),
                                "sk-fixture",
                            )
                            .model("fixture"),
                        )
                        .definition(
                            AgentDefinitionSpec::new()
                                .system_prompt("TEENY_CHANNEL_PROMPT")
                                .tools(ToolScopeSpec::Named(vec![
                                    "teeny_read".into(),
                                    "teeny_write".into(),
                                ])),
                        )
                        .tools(move |_| {
                            let (belt_reads, belt_writes, belt_origins) = (
                                Arc::clone(&belt_reads),
                                Arc::clone(&belt_writes),
                                Arc::clone(&belt_origins),
                            );
                            Box::pin(async move {
                                HostTurnTools::advertised(vec![
                                    Box::new(Probe {
                                        name: "teeny_read",
                                        level: PermissionLevel::ReadOnly,
                                        runs: belt_reads,
                                        origins: Arc::clone(&belt_origins),
                                    }),
                                    Box::new(Probe {
                                        name: "teeny_write",
                                        level: PermissionLevel::Write,
                                        runs: belt_writes,
                                        origins: belt_origins,
                                    }),
                                ])
                            })
                        }),
                )
                .expect("agent instantiates");

            // A bot can only be bound to an agent that exists.
            assert!(matches!(
                runtime
                    .channels()
                    .telegram(TelegramChannelSpec::new(TOKEN, "nobody")),
                Err(ChannelError::UnknownAgent(id)) if id == "nobody"
            ));

            let listener = runtime
                .channels()
                .telegram(TelegramChannelSpec::new(TOKEN, "teeny-chat").allow_everyone())
                .expect("the listener starts");
            assert_eq!(listener.channel(), "telegram");
            assert_eq!(listener.agent_id(), "teeny-chat");

            let mut sent = Vec::new();
            for _ in 0..80 {
                sent = sent_texts(&bot_api).await;
                if sent.iter().any(|text| text.contains("teeny says hi")) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if !sent.iter().any(|text| text.contains("teeny says hi")) {
                eprintln!(
                    "Telegram diagnostics: sent={sent:?}; Bot API requests={:#?}; model requests={:#?}",
                    bot_api.received_requests().await.unwrap_or_default(),
                    chat_requests(&provider).await
                );
            }
            assert!(
                sent.iter().any(|text| text.contains("teeny says hi")),
                "the agent's reply was posted to the chat: {sent:?}; Bot API requests: {:#?}; model requests: {:#?}",
                bot_api.received_requests().await.unwrap_or_default(),
                chat_requests(&provider).await
            );
            assert!(listener.is_running());
            listener.stop();

            assert_eq!(reads.load(Ordering::SeqCst), 1, "the read-only tool ran");
            assert_eq!(writes.load(Ordering::SeqCst), 0, "the write tool never ran");
            let origins = origins.lock().unwrap().clone();
            assert!(
                matches!(
                    origins.as_slice(),
                    [Some(AgentTurnOrigin::ExternalChannel { channel, sender: Some(sender), sender_name, .. })]
                        // The origin carries the stable Telegram user id (the
                        // envelope's `sender.id`), with the username as its name.
                        if channel == "telegram" && sender == "99"
                            && sender_name.as_deref() == Some("alice")
                ),
                "{origins:?}"
            );
            let requests = chat_requests(&provider).await;
            assert_eq!(requests.len(), 3);
            let first = String::from_utf8_lossy(&requests[0].body).to_string();
            assert!(first.contains("TEENY_CHANNEL_PROMPT"));
            assert!(first.contains("hello teeny"));
            let advertised = common::tool_names(&requests[0]);
            assert!(
                advertised.contains(&"teeny_read".to_string()),
                "{advertised:?}"
            );
            assert!(
                !advertised.contains(&"teeny_write".to_string()),
                "{advertised:?}"
            );

            drop(agent);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}
