use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::core::logging::CliLogDefault;

use super::protocol;

pub fn run_stdio_from_cli(args: &[String]) -> Result<()> {
    let mut verbose = false;

    for arg in args {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown mcp arg: {other}")),
        }
    }

    init_mcp_logging(verbose);

    log::debug!("[mcp_server] starting stdio MCP server");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async { run_stdio(tokio::io::stdin(), tokio::io::stdout()).await })?;
    Ok(())
}

/// Initialize logging for the stdio MCP server.
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
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = protocol::handle_json_line(trimmed).await {
            writer.write_all(response.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    log::debug!("[mcp_server] stdin closed; exiting");
    Ok(())
}

fn print_help() {
    // Use stderr so the help output never collides with the protocol stream,
    // matching the banner-suppression contract in `core/cli.rs` for the `mcp`
    // subcommand: stdout is reserved for JSON-RPC frames.
    eprintln!("Usage: openhuman-core mcp [-v|--verbose]");
    eprintln!();
    eprintln!("Start an opt-in stdio Model Context Protocol server.");
    eprintln!("The server exposes a curated read-only memory surface:");
    eprintln!("  memory.search");
    eprintln!("  memory.recall");
    eprintln!("  tree.read_chunk");
    eprintln!();
    eprintln!("Logging is written to stderr. JSON-RPC protocol messages are written to stdout.");
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
}
