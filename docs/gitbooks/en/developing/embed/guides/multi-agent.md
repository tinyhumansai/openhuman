---
description: "Share one runtime while configuring independent agent prompts and working folders."
icon: code
---

# Multiple agents

A runtime owns shared services and credentials. Each `AgentSpec` selects its prompt, model route, access policy, and `action_dir`. Internal agent state belongs to its agent home; the action directory is where acting tools operate.

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

The [complete two_agents example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/two_agents.rs) sends real turns and checks the provider requests: the analyst prompt does not contain the writer prompt. It also checks different action directories and agent homes. Run `cargo run -p openhuman-embed --example two_agents`.

Agents on one runtime share the runtime configuration and credential root. Separate action directories are useful for cooperating agents; use [SaaS profiles](saas-multi-tenant.md) for user isolation.

Runtime defaults can supply a provider and model settings; individual agents override selected fields. The [complete defaults_overrides example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/defaults_overrides.rs) verifies both routes and their transmitted temperature and token limits.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/defaults_overrides.rs#defaults_overrides -->

```rust
    let inherited = runtime.agent(AgentSpec::new("inherited"))?;
    let override_provider = support::provider("override reply").await;
    let overridden = runtime.agent(
        AgentSpec::new("overridden")
            .provider(support::route(&override_provider, "override-model"))
            .access(openhuman_embed::Access::readonly())
            .model_defaults(openhuman_embed::ModelDefaults {
                temperature: Some(0.8),
                max_tokens: Some(128),
                ..Default::default()
            }),
    )?;
    let inherited_reply = inherited.run("Hello default").await?;
    let overridden_reply = overridden.run("Hello override").await?;
    if support::offline() {
        assert_eq!(inherited_reply.reply, "hello from the stub");
        let requests = support::chat_requests(&provider).await;
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
        assert_eq!(body["model"], "fixture");
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["max_tokens"], 256);
    }
    assert_eq!(overridden_reply.reply, "override reply");
    let requests = support::chat_requests(&override_provider).await;
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    assert_eq!(body["model"], "override-model");
    assert_eq!(body["temperature"], 0.8);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(runtime.defaults().model.temperature, Some(0.4));
    println!("default and override routes answered independently");
```

<!-- END EMBED -->
