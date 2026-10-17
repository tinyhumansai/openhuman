---
description: "A turn is one native model/tool loop; a session is the durable identity that makes subsequent turns a conversation."
---

# Turns and sessions

`Agent::run` collects a turn. `Agent::turn` lets you set an explicit session, per-turn options, progress sink and cancellation before sending. Reuse the returned session identity for follow-up messages that should see the same transcript. Two calls on the same durable session remain distinct turn executions and receive distinct observation IDs.

`Agent::stream` returns a bounded `TurnStream`. Drain it while the turn is running: progress includes visible deltas and tool lifecycle notifications, followed by one `StreamEvent::Finished` containing the typed final result. Dropping the stream cancels its turn and recursive child work cooperatively.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Session store provider and shared progress/event infrastructure | Durable agent/thread identity, transcript and per-call cancellation |

## Verified example

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

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/streaming.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Attaching files

A host that already holds a file's bytes (an inbound photo from a channel webhook, an upload) attaches it with `Turn::attach(HostAttachment::new(bytes, mime).named(name))`, or several at once with `Turn::attachments`. Each file is staged into the agent's acting workspace and reaches the model ahead of the message: `image/*` as an image block, anything else as a document block. A model that cannot take a file natively gets the core's fallback instead (extracted text for PDF and Office documents, a vision readout for images, metadata otherwise). The `[multimodal]` and `[multimodal_files]` count and size limits apply, and a turn over them fails before any model call. Blank or `application/octet-stream` types are sniffed from the name and bytes.

No local path is read, so attachments are accepted whatever the turn's origin, including external-channel turns whose `[IMAGE:…]`/`[FILE:…]` text markers are refused. Only a runtime-owned `Agent` carries them: a turn on a caller-built runtime's orchestrator (`Core::agent`) is refused with `attachments_unsupported` rather than run without its files, because `inference.agent_chat` carries only a string.

## Behavior to account for

The stream example checks that deltas precede the final result and that collected and streaming replies agree. A cancellation token passed to a turn is preserved by streaming; cancellation unblocks forwarding even when the consumer has stopped draining a full buffer.

The harness derives transcript paths from the thread and agent IDs without a timestamp. Resumed sessions reuse their recorded system prompt and tool declarations; edits to prompts, skills or integrations apply to new sessions. Compaction seals a generation and opens a child instead of erasing history. See [observability](observability.md) for session IDs versus per-call turn IDs, and [API reference](../api-reference.md) for the session-store port.
