---
description: "The host composes infrastructure around one core; agents derive scoped contexts over its runtime."
---

# Embedding architecture

Embed is the library facade above `openhuman-core`. The TinyHumans layer installs the backend SDK transport and owns product login/session behavior; the RPC layer adds JSON-RPC, server listeners and shipped host boot. A Rust embedder can stay at the facade when it owns those concerns itself.

## Ownership boundaries

| Component | Owns |
| --- | --- |
| Your application | UI/HTTP transport, user authentication, supplied credentials, application tools and resource lifetimes |
| OpenHuman core and Embed facade | Runtime composition, per-agent context, policy, approvals, sandbox selection and typed domain calls |
| TinyAgents | Model/tool loop, parsing, dialects, durable sessions and transcript replay |
| TinyInference | Native model/provider contracts and generic ordered fallback |
| TinyTools | Shared tool traits and contract types |
| Storage and module libraries | Their backend/module implementations; OpenHuman adapts and orchestrates them |

The library chain is core → Embed → TinyHumans → RPC → app/CLI/TUI. Shipped hosts depend on the layer immediately below them and use its curated facades. Embed users should not build new integrations on doc-hidden `__host` internals.

## Runtime composition versus agent scope

Cargo features decide which implementations exist in an artifact. Runtime domains, services, tool groups and module selection narrow those compiled choices. Agent specs narrow the runtime further and add provider, access, prompt, skills, MCP and action-directory choices.

A turn dispatches through the agent's native context, not through JSON-RPC or an operator fallback. Recursive child contexts inherit the parent host overrides, including custom provider and hook composition, while independent sibling agents remain separately scoped.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/two_agents.rs#two_agents -->

```rust
    let analyst_dir = tempfile::tempdir()?;
    let writer_dir = tempfile::tempdir()?;
    let analyst = runtime.agent(
        AgentSpec::new("analyst")
            .system_prompt("ANALYST_PROMPT: summarize documents")
            .action_dir(analyst_dir.path())
            .access(openhuman_embed::Access::readonly()),
    )?;
    let writer = runtime.agent(
        AgentSpec::new("writer")
            .system_prompt("WRITER_PROMPT: compose explanations")
            .action_dir(writer_dir.path())
            .access(openhuman_embed::Access::full()),
    )?;
    assert_ne!(analyst.action_dir(), writer.action_dir());
    assert_ne!(analyst.home_dir(), writer.home_dir());
    assert_ne!(analyst.workspace_dir(), analyst.action_dir());
    assert!(!analyst.run("Analyze").await?.reply.is_empty());
    assert!(!writer.run("Explain").await?.reply.is_empty());
    if support::offline() {
        let requests = support::chat_requests(&provider).await;
        assert_eq!(requests.len(), 2);
        assert!(String::from_utf8_lossy(&requests[0].body).contains("ANALYST_PROMPT"));
        assert!(!String::from_utf8_lossy(&requests[0].body).contains("WRITER_PROMPT"));
        assert!(String::from_utf8_lossy(&requests[1].body).contains("WRITER_PROMPT"));
    }
    println!("two distinct prompts and action workspaces verified");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/two_agents.rs), including runtime construction, imports and fixture setup.

## Lifetime and isolation

An agent handle keeps the core alive after the runtime handle is dropped. Explicit removal marks that instance removed, cancels turns and callbacks, denies parked approvals and clears its state/MCP registration. The ID remains reserved through cleanup, including natural last-handle destruction and cancellation of a started removal. Pointer-checked release prevents an old retained instance from removing a newer registry entry.

Agent separation is not credential separation: all ordinary runtime agents share the operator's config path and credential root. Use [profiles](concepts/profiles-saas.md) for independently authenticated users. See [storage architecture](../architecture/storage.md) for scope/driver ownership and [agent harness architecture](../architecture/agent-harness.md) for session execution.
