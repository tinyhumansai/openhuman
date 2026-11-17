---
description: "Runtime metadata events track lifecycle and approvals; turn streams carry per-turn progress and the final result."
---

# Observability

Subscribe with `Runtime::events` before starting work you want to observe. Each event has a sequence number and runtime identity, with an agent ID and per-call turn ID where applicable. Repeated calls on the same durable session receive different turn IDs; the session ID is not the observation ID.

Runtime events contain lifecycle metadata, not user prompts or model output. Detailed text/tool progress belongs to the explicitly requested turn stream. A bounded event receiver reports lag explicitly rather than silently claiming that it delivered every event.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Runtime sequence, content-safe lifecycle feed and registered-agent filter | Per-call turn correlation and opt-in text/tool progress stream |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/runtime_events.rs#runtime_events -->

```rust
    let mut events = runtime.events();
    let count = Arc::new(AtomicUsize::new(0));
    runtime.post_turn_hook("live-counter", Some(Arc::new(Counter(count.clone()))));
    let agent = runtime.agent(AgentSpec::new("observed"))?;
    assert_eq!(agent.run("private payload one").await?.reply, "event reply");
    support::eventually("runtime hook", || {
        (count.load(Ordering::SeqCst) == 1).then_some(())
    })
    .await;
    runtime.post_turn_hook("live-counter", None);
    assert_eq!(agent.run("private payload two").await?.reply, "event reply");
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "removed hook stops observing turns"
    );
    runtime.remove_agent(agent.id()).await?;
    let mut previous = 0;
    let (mut added, mut started, mut ended) = (0, 0, 0);
    loop {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await??;
        assert!(event.sequence > previous);
        previous = event.sequence;
        assert_eq!(event.agent_id.as_deref(), Some("observed"));
        assert!(!serde_json::to_string(&event)?.contains("private payload"));
        match event.kind {
            RuntimeEventKind::AgentAdded => added += 1,
            RuntimeEventKind::TurnStarted => {
                assert!(event.turn_id.is_some());
                started += 1;
            }
            RuntimeEventKind::TurnEnded { success } => {
                assert!(success);
                ended += 1;
            }
            RuntimeEventKind::AgentRemoved => break,
            _ => {}
        }
    }
    assert_eq!((added, started, ended), (1, 2, 2));
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/runtime_events.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example checks ordered add/start/end/remove events and confirms that private prompt text is absent from serialized metadata. Approval events are filtered to agents registered on this runtime. Correlation follows the effective origin thread; a request's later decision keeps its original turn correlation after the turn has ended.

When simultaneous turns share an origin thread, approval correlation can be ambiguous and its turn ID is absent rather than guessed. Handle receiver lag by refreshing state, and do not send text from `AgentProgress` to content-free analytics. See [turns and sessions](turns-sessions.md), [approvals](access-approvals.md), and [testing](testing.md).
