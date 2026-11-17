//! Title: Embed an agent behind a host-owned HTTP server
//! Summary: Embed an agent behind a host-owned HTTP server.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
//! Feature: default

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: server
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
    // ANCHOR_END: server
    support::passed("server");
    Ok(())
}
