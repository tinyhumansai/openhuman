---
description: "ProfileRuntime isolates authenticated users by workspace, credentials, conversations and event streams."
---

# Profiles and SaaS

Ordinary runtime agents share an operator's configuration path and credentials. Use `ProfileRuntime` when one server serves different authenticated users who must not share those resources. Provision a profile, install its credential, and retain a `ProfileHandle` while serving that user's requests.

Profile mode claims the process separately from ordinary `Runtime` and `Harness`. Its root must satisfy the boot guard: absolute, existing, outside the operator's workspace and not world-writable. Remove single-user workspace/token environment variables before booting a profile server.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| One profile host, shared compiled core and managed backend transport | Per-profile credential, workspace, threads, memory and event access |

## Verified example

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

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/profiles.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example creates Alice and Bob with the same thread name and verifies that each transcript contains only its own note. Handles keep their profiles open; release/idle eviction can close a profile only after those handles are dropped.

Production managed calls need the TinyHumans transport installed by the host layer. `ProfileHandle::call` exposes the reviewed user surface rather than arbitrary operator controllers. Profiles do not make a shared file-system action directory safe automatically; provision external application resources per user too. See [SaaS deployment guide](../guides/saas-multi-tenant.md) and [SaaS architecture](../../saas-profiles.md).
