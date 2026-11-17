---
description: "Invoke agents from a host-owned HTTP service without adding RPC to Embed."
icon: code
---

# Deploy a server

Your HTTP host can keep a runtime alive and invoke its agents directly. The executable example binds a loopback listener, handles one request, and sends back the agent's reply. It exercises the request path without requiring an RPC server dependency in `openhuman-embed`.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/server.rs#server -->

```rust
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let agent = runtime.agent(
        AgentSpec::new("http-agent").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    // A real host should use its HTTP framework and authenticate requests.
    // This single-request fixture keeps the library dependency direction intact.
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = vec![0; 4096];
        let length = socket.read(&mut request).await?;
        assert!(String::from_utf8_lossy(&request[..length]).starts_with("GET /hello "));
        let reply = agent.run("Say hello").await?.reply;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply.len(),
            reply
        );
        socket.write_all(response.as_bytes()).await?;
        Ok::<_, anyhow::Error>(())
    });
    let reply = reqwest::get(format!("http://{address}/hello"))
        .await?
        .error_for_status()?
        .text()
        .await?;
    assert!(!reply.is_empty());
    if support::offline() {
        assert_eq!(reply, "hello from the stub");
    }
    server.await??;
    println!("host HTTP request reached the in-process agent");
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example server`. The [complete server example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/server.rs) checks the HTTP response against the deterministic provider reply.

Use your HTTP framework for a deployed service. The example's one-request parser is deliberately small; a deployed host must handle authentication, request limits, cancellation, and shutdown. Select agents from authenticated host state. For multiple users, use [SaaS profiles](saas-multi-tenant.md).

If your host needs OpenHuman's existing JSON-RPC server, depend on `openhuman-rpc`, the layer above Embed. Adding that dependency to Embed would reverse the crate chain. [Channels](../integrations/channels.md) provide another way to deliver messages to bound agents.
