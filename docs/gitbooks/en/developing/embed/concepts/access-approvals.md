---
description: "Access configures turn identity and permission policy; the host chooses how parked requests receive decisions."
---

# Access and approvals

`Access` groups permission level, origin, standing grants and trusted roots. Choose an origin and tier together: a trusted local host request and an external channel message are different security contexts. `Access::full()` configures both access fields. A sandbox is an additional execution boundary, configured through `AgentDefinitionSpec`.

The core autonomy policy is disabled by default. Set `config.autonomy.enabled` when you require its command classification, approval gate, allowlist and budget enforcement. Platform isolation and the credential/system-root safety floor have separate responsibilities; a tool group filter is not an operating-system sandbox.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Autonomy policy defaults and installed approval gate | Origin, access tier, grants, trusted roots and scoped decisions |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/approvals.rs#approvals -->

```rust
    let scratch = tempfile::tempdir()?;
    let marker = scratch.path().join("approved-marker");
    let write_provider = support::scripted_provider(
        vec![support::tool_call_completion(
            "shell",
            &serde_json::json!({"command": format!("touch {}", marker.display())}).to_string(),
        )],
        "approved",
    )
    .await;
    let agent = runtime.agent(
        AgentSpec::new("supervised")
            .provider(support::route(&write_provider, "fixture"))
            .action_dir(scratch.path())
            .access(
                openhuman_embed::Access::supervised()
                    .origin(openhuman_embed::AgentTurnOrigin::WebChat {
                        thread_id: "demo".into(),
                        client_id: "host".into(),
                        request_id: None,
                    })
                    .auto_approve(Vec::<String>::new())
                    .auto_approve_all(false)
                    .trust(
                        scratch.path().display().to_string(),
                        openhuman_embed::TrustedAccess::ReadWrite,
                    ),
            )
            .definition(
                AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])),
            ),
    )?;
    let pending_agent = agent.clone();
    let turn = tokio::spawn(async move { pending_agent.run("Create a marker").await });
    let pending = support::eventually("approval", || {
        agent.approvals().pending().ok()?.into_iter().next()
    })
    .await;
    assert!(!marker.exists(), "execution waits for the host decision");
    assert_eq!(pending.agent_id.as_deref(), Some("supervised"));
    agent.approvals().decide(
        &pending.request_id,
        openhuman_embed::ApprovalDecision::ApproveOnce,
    )?;
    assert_eq!(turn.await??.reply, "approved");
    assert!(marker.exists());
    assert!(agent.approvals().pending()?.is_empty());
    println!("parked write executed only after approval");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/approvals.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The verified example enables autonomy before building the runtime. It starts a real shell write, observes that no marker exists while the call is parked, and creates it only after `ApproveOnce`. `agent.approvals()` lists and decides only that instance's requests. A removed instance returns an empty pending list and rejects decisions even if the same public ID has been reused.

For asynchronous integration use `AgentSpec::approval_handler` or retain the handle returned by `Agent::handle_approvals`. Dropping a subscription cancels its pending decision future; it does not silently approve the request. Agent removal cancels both builder and live callbacks. Continue with the [approval callback example](../cookbook.md#polling-and-callback-approvals-with-cancellation) and [sandbox guide](../guides/multi-agent.md).
