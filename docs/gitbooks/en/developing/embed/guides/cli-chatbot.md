---
description: "Collect a reply or consume progress events in a Rust command-line host."
icon: code
---

# CLI chatbot

Keep one `Runtime` alive for the process and create an `Agent` for the conversation. The executable example starts with a single message, checks the reply, and prints it. A CLI input loop can send subsequent messages through the same agent; choose a stable thread identity when your host exposes separate conversations.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/run_turn.rs#run_turn -->

```rust
    let agent = runtime.agent(
        AgentSpec::new("hello").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    let outcome = agent.run("Say hello").await?;
    assert!(!outcome.reply.is_empty());
    if support::offline() {
        assert_eq!(outcome.reply, "hello from the stub");
    }
    println!("{}", outcome.reply);
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example run_turn` from the repository root. It uses a loopback provider and temporary state. The [complete run_turn example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/run_turn.rs) includes imports, runtime construction, and offline setup.

For incremental output, consume `agent.stream(...)` until `StreamEvent::Finished`. Progress includes text deltas; the final event carries the same outcome shape as `run`. Dropping the stream cancels the underlying turn.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/streaming.rs#streaming -->

```rust
    let agent = runtime.agent(AgentSpec::new("streamer"))?;
    let mut stream = agent.stream("Hello streaming");
    let mut count = 0;
    let mut deltas = String::new();
    let mut finished = None;
    while let Some(event) = stream.recv().await {
        match event {
            openhuman_embed::StreamEvent::Progress(progress) => {
                println!("{progress:?}");
                if let openhuman_embed::AgentProgress::TextDelta { delta, .. } = progress {
                    assert!(finished.is_none(), "text deltas precede Finished");
                    deltas.push_str(&delta);
                }
                count += 1;
            }
            openhuman_embed::StreamEvent::Finished(result) => finished = Some(result?),
        }
    }
    let outcome = finished.expect("stream ends with an outcome");
    assert!(!outcome.reply.is_empty());
    assert!(
        !deltas.is_empty(),
        "stream must publish visible text before Finished"
    );
    assert_eq!(deltas, outcome.reply);
    if support::offline() {
        let regular = agent.run("Hello streaming").await?;
        assert_eq!(
            outcome.reply, regular.reply,
            "streaming and collected turns agree"
        );
    }
    println!("progress events: {count}");
```

<!-- END EMBED -->

The [complete streaming example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/streaming.rs) checks that text arrives before completion and that the offline streamed reply equals a collected reply. [Provider configuration](../integrations/openai-compatible.md) covers explicit live inference.
