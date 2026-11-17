---
description: "Bind an incoming channel to an agent and verify the delivered reply."
icon: code
---

# Channels

Enable the `channels` Cargo feature to use channel adapters. Your host configures the channel credential and binds incoming messages to an agent. In the Telegram example, the Bot API endpoint is a loopback stub, so updates and replies exercise the adapter without contacting Telegram.

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

Run `cargo run -p openhuman-embed --example telegram --features channels`. The [complete telegram example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/telegram.rs) receives a stub update, runs the bound agent, and checks that the resulting reply was posted to the expected chat id.

Keep channel credentials in host configuration and map messages to the intended identity and conversation. A provider route controls inference; it does not configure a channel or provide a hosted relay. [Managed backend setup](tinyhumans-managed.md) explains that separate transport responsibility.

For a live Telegram host, supply the real Bot API configuration through your application's channel setup. This offline example deliberately has no live channel environment switch. [Deploy a server](../guides/deploy-server.md) covers a host-owned HTTP alternative.
