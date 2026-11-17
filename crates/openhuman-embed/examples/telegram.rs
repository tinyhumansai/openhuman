//! Title: Telegram channel answered by its bound agent via a local Bot API
//! Summary: Telegram channel answered by its bound agent via a local Bot API.
//! Run: offline with loopback stubs; no live path.
//! Feature: channels

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    // The channel runtime builds its default chat model from the config's
    // managed provider before any bound agent runs. The core carries no hosted
    // provider, so register one that sends inference to the stub.
    support::PointedInference::install(&format!("{}/v1", provider.uri()));
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .api_key("sk-fixture")
        .provider(support::route(&provider, "fixture"))
        .build()
        .await?;
    // ANCHOR: telegram
    let bot = wiremock::MockServer::start().await;
    let token = "123456:EXAMPLE";
    let updates = format!("/bot{token}/getUpdates");
    wiremock::Mock::given(wiremock::matchers::path(updates.clone())).respond_with(
        wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":[{
            "update_id":1,"message":{"message_id":1,"date":0,"text":"hello channel",
            "from":{"id":99,"is_bot":false,"username":"alice"},"chat":{"id":4242,"type":"private"}}
        }]}))).up_to_n_times(1).with_priority(1).mount(&bot).await;
    wiremock::Mock::given(wiremock::matchers::path(updates))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"result":[]}))
                .set_delay(std::time::Duration::from_millis(100)),
        )
        .with_priority(2)
        .mount(&bot)
        .await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"result":{
            "message_id":2,"id":1,"is_bot":true,"username":"example_bot"}})),
        )
        .with_priority(10)
        .mount(&bot)
        .await;
    std::env::set_var("OPENHUMAN_TELEGRAM_BOT_API_BASE", bot.uri());
    let agent = runtime.agent(
        AgentSpec::new("channel-agent").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    let listener = runtime.channels().telegram(
        openhuman_embed::TelegramChannelSpec::new(token, agent.id()).allowed_users(["alice"]),
    )?;
    let mut sent = false;
    for _ in 0..200 {
        let requests = bot.received_requests().await.unwrap_or_default();
        sent = requests.iter().any(|request| {
            request.url.path().ends_with("/sendMessage")
                && serde_json::from_slice::<serde_json::Value>(&request.body).is_ok_and(|body| {
                    body["chat_id"].to_string().contains("4242")
                        && body["text"]
                            .as_str()
                            .is_some_and(|text| text.contains("hello from the stub"))
                })
        });
        if sent {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    listener.stop();
    assert!(
        sent,
        "the bound agent must send a reply to the stub Bot API"
    );
    std::env::remove_var("OPENHUMAN_TELEGRAM_BOT_API_BASE");
    println!("Telegram update answered in the correct chat");
    // ANCHOR_END: telegram
    support::passed("telegram");
    Ok(())
}
