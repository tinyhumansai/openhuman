use anyhow::{bail, Result};
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::core::logging::CliLogDefault;

use super::http::{run_http, HttpServerConfig};
use super::{protocol, session::McpSession};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpTransport {
    Stdio,
    Http,
}

pub fn run_stdio_from_cli(args: &[String]) -> Result<()> {
    let mut verbose = false;
    let mut transport = McpTransport::Stdio;
    let mut bind_host = "127.0.0.1".to_string();
    let mut port: u16 = 9300;
    let mut auth_token: Option<String> = None;

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-v" | "--verbose" => {
                verbose = true;
                index += 1;
            }
            "--transport" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --transport"))?;
                transport = match value.as_str() {
                    "stdio" => McpTransport::Stdio,
                    "http" => McpTransport::Http,
                    other => bail!("unknown --transport value `{other}` (expected stdio or http)"),
                };
                index += 2;
            }
            "--host" => {
                bind_host = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --host"))?
                    .clone();
                index += 2;
            }
            "--port" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = raw
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid --port value `{raw}`"))?;
                index += 2;
            }
            "--auth-token" => {
                let token = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --auth-token"))?;
                if token.trim().is_empty() {
                    bail!("--auth-token must not be empty");
                }
                auth_token = Some(token.trim().to_string());
                index += 2;
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => bail!("unknown mcp arg: {other}"),
        }
    }

    init_mcp_logging(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    match transport {
        McpTransport::Stdio => {
            log::debug!("[mcp_server] starting stdio MCP server");
            rt.block_on(async { run_stdio(tokio::io::stdin(), tokio::io::stdout()).await })?;
        }
        McpTransport::Http => {
            let bind_addr: SocketAddr = format!("{bind_host}:{port}").parse().map_err(|err| {
                anyhow::anyhow!("invalid bind address `{bind_host}:{port}`: {err}")
            })?;
            log::debug!(
                "[mcp_server] starting HTTP/SSE MCP server bind={bind_addr} auth={}",
                auth_token.is_some()
            );
            rt.block_on(run_http(HttpServerConfig {
                bind_addr,
                auth_token,
            }))?;
        }
    }
    Ok(())
}

/// Initialize logging for the MCP server.
///
/// MCP servers run as subprocesses of clients (Claude Desktop, Cursor, …) which
/// surface the server's stderr to the user when something goes wrong. We
/// therefore always install the tracing subscriber — otherwise `log::error!` /
/// `log::warn!` events get silently dropped and field-debugging requires
/// re-running with `--verbose`.
///
/// Default level is `warn` to keep the stderr stream quiet under normal use
/// while still surfacing problems; `--verbose` promotes it to `debug` so
/// `[mcp_server]` traces become visible. A user-set `RUST_LOG` always wins.
fn init_mcp_logging(verbose: bool) {
    if std::env::var_os("RUST_LOG").is_none() {
        let level = if verbose { "debug" } else { "warn" };
        std::env::set_var("RUST_LOG", level);
    }
    crate::core::logging::init_for_cli_run(verbose, CliLogDefault::Global);
}

pub async fn run_stdio<R, W>(reader: R, mut writer: W) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut session = McpSession::default();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = protocol::handle_json_line_with_session(trimmed, &mut session).await
        {
            writer.write_all(response.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    log::debug!("[mcp_server] stdin closed; exiting");
    Ok(())
}

fn print_help() {
    eprintln!("Usage: openhuman-core mcp [options]");
    eprintln!();
    eprintln!("Start an opt-in Model Context Protocol server.");
    eprintln!();
    eprintln!("Transports:");
    eprintln!("  (default)           stdio — newline-delimited JSON-RPC on stdin/stdout");
    eprintln!("  --transport http    Streamable HTTP + SSE on a local bind address");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -v, --verbose           Log at debug level on stderr");
    eprintln!("  --transport <stdio|http>  Transport (default: stdio)");
    eprintln!("  --host <addr>           Bind host for HTTP transport (default: 127.0.0.1)");
    eprintln!("  --port <port>           Bind port for HTTP transport (default: 9300)");
    eprintln!("  --auth-token <token>    Require Authorization: Bearer <token> on HTTP requests");
    eprintln!();
    eprintln!("Tools exposed (stdio and HTTP):");
    eprintln!("  core.list_tools, core.tool_instructions");
    eprintln!("  agent.list_subagents, agent.run_subagent");
    eprintln!("  memory.search, memory.recall, tree.read_chunk, tree.browse,");
    eprintln!("  tree.top_entities, tree.list_sources");
    eprintln!();
    eprintln!("Logging is written to stderr. Stdio protocol messages use stdout only.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt};

    #[tokio::test]
    async fn stdio_loop_writes_one_line_per_response() {
        let (mut client_write, server_read) = duplex(4096);
        let (server_write, mut client_read) = duplex(4096);

        let server = tokio::spawn(async move { run_stdio(server_read, server_write).await });

        client_write
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"ping"}
"#,
            )
            .await
            .unwrap();
        drop(client_write);

        let mut output = String::new();
        client_read.read_to_string(&mut output).await.unwrap();
        server.await.unwrap().unwrap();

        let response: serde_json::Value =
            serde_json::from_str(output.trim()).expect("json response");
        assert_eq!(response["id"], 1);
        assert!(response["result"].is_object());
    }

    #[test]
    fn cli_help_exits_zero() {
        assert!(run_stdio_from_cli(&["--help".into()]).is_ok());
    }

    #[test]
    fn cli_verbose_advances_to_next_arg() {
        assert!(run_stdio_from_cli(&["--verbose".into(), "--help".into()]).is_ok());
    }
}
