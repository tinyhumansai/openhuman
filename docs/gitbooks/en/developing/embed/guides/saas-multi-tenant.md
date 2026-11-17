---
description: "Isolate user credentials, workspaces, and conversation history with ProfileRuntime."
icon: code
---

# SaaS with multiple tenants

Use `ProfileRuntime` for a process serving multiple users. Provision each profile, install that profile's credential, and open a handle to its workspace. Reusing a thread id in two profiles does not share their conversation history.

The complete example prepares a temporary SaaS root and initializes the keyring at `SaasConfig::operator_config().workspace_dir` before boot. SaaS validates the operator root, service token, permitted domains, and environment. A process-wide `OPENHUMAN_WORKSPACE` override is rejected. A SaaS core must be the only core in its process.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/profiles.rs#profiles -->

```rust
    let profiles = openhuman_embed::ProfileRuntime::build(config).await?;
    let mut handles = Vec::new();
    for (user, text) in [("alice", "Alice private note"), ("bob", "Bob private note")] {
        let profile = profiles.provision(user).await?;
        profiles
            .set_credential(
                &profile.profile_id,
                openhuman_embed::profiles::ProfileCredentialKind::ApiKey,
                &format!("{user}-fixture"),
            )
            .await?;
        let handle = profiles.open(user).await?;
        let reply = handle.chat("shared-thread", text).await?;
        assert!(reply.text.contains(text), "{}", reply.text);
        handles.push(handle);
    }
    assert_ne!(handles[0].workspace_dir(), handles[1].workspace_dir());
    for (index, own, other) in [
        (0, "Alice private note", "Bob private note"),
        (1, "Bob private note", "Alice private note"),
    ] {
        let messages = handles[index].messages("shared-thread").await?;
        let text = serde_json::to_string(&messages)?;
        assert!(text.contains(own));
        assert!(!text.contains(other));
    }
    drop(handles);
    profiles.shutdown().await;
    println!("two profiles keep same-thread conversations separate");
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example profiles`. The [complete profiles example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/profiles.rs) installs a loopback backend transport, chats as Alice and Bob, then checks distinct workspaces and same-thread history isolation. No live account or backend is contacted.

Authenticate users in your host before selecting a profile. Keep the operator's configuration and gateway credential separate from user credentials. [Managed inference](../integrations/tinyhumans-managed.md) explains who provides the backend transport.
