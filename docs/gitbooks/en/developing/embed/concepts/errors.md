---
description: "Typed errors distinguish build availability, expected domain states, host setup failures and removed agent instances."
---

# Errors

Match the error type at its boundary: `RuntimeError` describes boot/configuration, `AgentError` describes registration/layout, and `CoreError` describes typed facade calls and turns. A missing compiled or selected domain is `CoreError::Unavailable`; it is a capability decision the host can reflect in its UI.

A structured `CoreError::Domain` carries a stable kind and `expected_user_state`. Expected stale-session or missing-resource states should be shown as notices rather than treated as internal faults. `Decode` signals a facade/domain shape mismatch; retrying unchanged input does not repair it.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Boot errors, unavailable domains and shared transport setup | Registration errors, scoped domain failures and AgentRemoved on retained handles |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/sandbox_access.rs#sandbox_access -->

```rust
    let scratch = tempfile::tempdir()?;
    let marker = scratch.path().join("attempted-write");
    let attempted_write = support::tool_call_completion(
        "write_file",
        &serde_json::json!({
            "path": marker.display().to_string(), "content": "example"
        })
        .to_string(),
    );
    let read_provider = support::scripted_provider(vec![attempted_write], "write refused").await;
    let read = runtime.agent(
        AgentSpec::new("read")
            .provider(support::route(&read_provider, "fixture"))
            .action_dir(scratch.path())
            .access(openhuman_embed::Access::readonly())
            .definition(
                AgentDefinitionSpec::new().sandbox(openhuman_embed::SandboxModeSpec::ReadOnly),
            ),
    )?;
    let full = runtime.agent(AgentSpec::new("full").access(openhuman_embed::Access::full()))?;
    assert_ne!(read.config().autonomy.level, full.config().autonomy.level);
    assert!(!read.run("Write a marker").await?.reply.is_empty());
    assert!(
        !marker.exists(),
        "readonly sandbox must refuse the attempted write"
    );
    let requests = support::chat_requests(&read_provider).await;
    assert!(!support::tool_names(&requests[0]).contains(&"write_file".to_string()));
    println!("readonly sandbox refused a model-requested write");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/sandbox_access.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example's refused write is handled as a tool result while the turn can still finish with a reply. Do not assume every rejected tool action is a failed turn. Conversely, a finished stream can contain an error; always inspect `StreamEvent::Finished`.

Bearer routes refuse non-TLS remote endpoints before sending credentials. `BACKEND_UNAVAILABLE` means the embed-only host has no backend transport installed; check the host layer before rotating keys. Agent removal makes later turns on retained handles return `AgentRemoved`, even when its ID now belongs to another instance. See [troubleshooting](../troubleshooting.md) and [provider integration](../integrations/openai-compatible.md).
