---
description: "Completer sends stateless model requests without booting a Runtime or entering the agent tool loop."
---

# Completer

Use `Completer` for one-shot text or structured generation when you do not need an agent, its tool loop or persistent transcript. It takes an explicit route and a `CompletionRequest`; the caller supplies the model and messages for every request.

A standalone completer does not claim the process runtime slot. An agent-backed completion uses the agent's effective configuration but still has no implicit durable session. Keep conversation state in your application if you need several completion calls to form a conversation.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| No runtime required for a standalone routed Completer | Optional effective agent configuration; no implicit tools or transcript |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/completer.rs#completer -->

```rust
    let route = support::example_provider(&provider)?;
    let completer = openhuman_embed::Completer::new(route.route().expect("explicit route").clone());
    let response = completer
        .complete(openhuman_embed::CompletionRequest::new(
            route.model_id().expect("explicit model"),
            vec![openhuman_embed::ChatMessage::user("Say hello")],
        ))
        .await?;
    assert!(!response.text.is_empty());
    if support::offline() {
        assert_eq!(response.text, "completion reply");
        assert_eq!(support::chat_requests(&provider).await.len(), 1);
    }
    println!("{}", response.text);
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/completer.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example checks that exactly one inference request was made and verifies the returned text. It uses a local provider fixture by default; the source contains the explicit opt-in live route path.

Choose `Agent` when a model should call tools, resume a durable thread or execute with an agent's access context. Choose the structured completion helpers when the host only needs a validated data result. For routes and transport failures see [providers](providers.md) and [errors](errors.md).

## Timeouts and fallback

Set `CompletionRequest::timeout_ms(120_000)` to bound each physical provider HTTP request to 120 seconds. The optional field defaults to unset and is omitted from serialized requests when absent, retaining the provider's transport defaults. Each structured repair attempt receives its own physical timeout; this is not a total completion or ladder deadline.

A physical timeout is an RPC transport failure, so an explicit `CompletionLadder` can try its next route. By contrast, `Completer::timeout(duration)` bounds the entire logical completion, including repair attempts, and returns terminal `DeadlineExceeded`. Cancellation also stops the ladder without fallback. You can use both bounds when the host needs separate physical and logical limits.

A timed-out physical attempt has unknown reported cost. Ladder aggregate cost stays unknown, and a budgeted call keeps its conservative reservation if no authoritative usage arrived. The remaining budget may therefore refuse a fallback before it sends another request. See the [routing guide](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/ROUTING.md) for the explicit ladder API.
