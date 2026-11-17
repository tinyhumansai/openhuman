---
description: "A channel listener binds external messages to one registered agent and keeps transport lifecycle under host control."
---

# Channels

`Runtime::channels` creates listeners addressed to a live agent ID. A listener owns its transport task; dropping or stopping it ends that listener. The agent supplies the prompt, provider and tools used for every incoming message instead of handing messages to an operator agent.

The named Embed `channels` feature is enabled by default. The runtime must also allow the channel family. Create listeners inside the Tokio runtime after their agents have been registered.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Channel implementation, listener registry and runtime domain ceiling | Bound agent, conversation identity and external-message access context |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/telegram.rs#telegram -->

```rust
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
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/telegram.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The verified Telegram example uses a local Bot API stub, so the channel test does not contact Telegram. Its live mode requires explicit example credentials and remains opt-in.

External messages use `ExternalChannel` origin and a read-only cap. Model-requested writes and other external effects are not parked for approval from the sender. Keep public-chat tools read-only and perform authorized side effects in host code or a separately configured job. When the bound agent is removed, channel work fails as unavailable instead of falling back to the operator. See [channel integration](../integrations/channels.md).
