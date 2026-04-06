//! `openhuman text-input` — standalone CLI for text input intelligence.
//!
//! Reads, inserts, and previews text in the OS-focused input field without
//! starting the full desktop app. Useful for testing autocomplete, voice
//! input, and accessibility integration from a terminal.
//!
//! Usage:
//!   openhuman text-input run       [--port <u16>] [-v]
//!   openhuman text-input read      [-v] [--bounds]
//!   openhuman text-input insert    <text> [-v]
//!   openhuman text-input ghost     <text> [--ttl <ms>] [-v]
//!   openhuman text-input dismiss   [-v]

use anyhow::Result;

/// Entry point for `openhuman text-input <subcommand>`.
pub fn run_text_input_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "run" => run_server(&args[1..]),
        "read" => run_read(&args[1..]),
        "insert" => run_insert(&args[1..]),
        "ghost" => run_ghost(&args[1..]),
        "dismiss" => run_dismiss(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown text-input subcommand '{other}'. Run `openhuman text-input --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Option parsing
// ---------------------------------------------------------------------------

struct CliOpts {
    port: u16,
    verbose: bool,
    ttl_ms: u32,
    include_bounds: bool,
}

fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut port: u16 = 7798;
    let mut verbose = false;
    let mut ttl_ms: u32 = 3000;
    let mut include_bounds = false;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --port: {e}"))?;
                i += 2;
            }
            "--ttl" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --ttl"))?;
                ttl_ms = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --ttl: {e}"))?;
                i += 2;
            }
            "--bounds" => {
                include_bounds = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                rest.push(args[i].clone());
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }

    Ok((
        CliOpts {
            port,
            verbose,
            ttl_ms,
            include_bounds,
        },
        rest,
    ))
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// `openhuman text-input run` — start a minimal JSON-RPC server.
fn run_server(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman text-input run [--port <u16>] [-v]");
        println!();
        println!("Start a lightweight JSON-RPC server exposing text input RPC methods.");
        println!();
        println!("  --port <u16>     Listen port (default: 7798)");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    crate::core::logging::init_for_cli_run(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let app = build_router();

        let bind_addr = format!("127.0.0.1:{}", opts.port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

        log::info!("[text-input-cli] ready — http://{bind_addr}/rpc (JSON-RPC 2.0)");

        eprintln!();
        eprintln!("  Text input dev server listening on http://{bind_addr}");
        eprintln!("  JSON-RPC endpoint: POST http://{bind_addr}/rpc");
        eprintln!("  Health check:      GET  http://{bind_addr}/health");
        eprintln!("  Press Ctrl+C to stop.");
        eprintln!();

        axum::serve(listener, app).await?;
        Ok(())
    })
}

/// `openhuman text-input read` — one-shot read of the focused field.
fn run_read(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman text-input read [--bounds] [-v]");
        println!();
        println!("Read the currently focused text input field and print JSON to stdout.");
        println!();
        println!("  --bounds         Include element bounds in the output");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let params = crate::openhuman::text_input::ReadFieldParams {
            include_bounds: Some(opts.include_bounds),
        };
        let outcome = crate::openhuman::text_input::rpc::read_field(params).await.map_err(|e| anyhow::anyhow!(e))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

/// `openhuman text-input insert <text>` — insert text into the focused field.
fn run_insert(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman text-input insert <text> [-v]");
        println!();
        println!("Insert text into the currently focused input field.");
        println!();
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    init_quiet_logging(opts.verbose);

    let text = rest.join(" ");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let params = crate::openhuman::text_input::InsertTextParams {
            text,
            validate_focus: None,
            expected_app: None,
            expected_role: None,
        };
        let outcome = crate::openhuman::text_input::rpc::insert_text(params).await.map_err(|e| anyhow::anyhow!(e))?;
        if outcome.value.inserted {
            eprintln!("  Text inserted successfully.");
        } else {
            eprintln!(
                "  Insert failed: {}",
                outcome.value.error.as_deref().unwrap_or("unknown error")
            );
            std::process::exit(1);
        }
        Ok(())
    })
}

/// `openhuman text-input ghost <text>` — show ghost text overlay.
fn run_ghost(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman text-input ghost <text> [--ttl <ms>] [-v]");
        println!();
        println!("Show ghost text overlay near the focused input field.");
        println!();
        println!("  --ttl <ms>       Auto-dismiss after N milliseconds (default: 3000)");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    init_quiet_logging(opts.verbose);

    let text = rest.join(" ");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let params = crate::openhuman::text_input::ShowGhostTextParams {
            text,
            ttl_ms: Some(opts.ttl_ms),
            bounds: None,
        };
        let outcome = crate::openhuman::text_input::rpc::show_ghost(params).await.map_err(|e| anyhow::anyhow!(e))?;
        if outcome.value.shown {
            eprintln!("  Ghost text shown (ttl={}ms).", opts.ttl_ms);
        } else {
            eprintln!(
                "  Show ghost failed: {}",
                outcome.value.error.as_deref().unwrap_or("unknown error")
            );
            std::process::exit(1);
        }
        Ok(())
    })
}

/// `openhuman text-input dismiss` — dismiss the ghost text overlay.
fn run_dismiss(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman text-input dismiss [-v]");
        println!();
        println!("Dismiss the ghost text overlay.");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let outcome = crate::openhuman::text_input::rpc::dismiss_ghost().await.map_err(|e| anyhow::anyhow!(e))?;
        if outcome.value.dismissed {
            eprintln!("  Ghost text dismissed.");
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Minimal HTTP router
// ---------------------------------------------------------------------------

fn build_router() -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/health", get(health))
        .route("/rpc", post(rpc))
}

async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({ "ok": true, "mode": "text-input-dev" }))
}

async fn rpc(
    axum::Json(req): axum::Json<crate::core::types::RpcRequest>,
) -> axum::response::Response {
    use crate::core::types::{RpcError, RpcFailure, RpcSuccess};
    use axum::response::IntoResponse;

    let id = req.id.clone();
    let state = crate::core::jsonrpc::default_state();

    match crate::core::jsonrpc::invoke_method(state, req.method.as_str(), req.params).await {
        Ok(value) => (
            axum::http::StatusCode::OK,
            axum::Json(RpcSuccess {
                jsonrpc: "2.0",
                id,
                result: value,
            }),
        )
            .into_response(),
        Err(message) => (
            axum::http::StatusCode::OK,
            axum::Json(RpcFailure {
                jsonrpc: "2.0",
                id,
                error: RpcError {
                    code: -32000,
                    message,
                    data: None,
                },
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::core::logging::init_for_cli_run(verbose);
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_help() {
    println!("openhuman text-input — text input intelligence\n");
    println!("Usage:");
    println!("  openhuman text-input run       [--port <u16>] [-v]");
    println!("  openhuman text-input read      [--bounds] [-v]");
    println!("  openhuman text-input insert    <text> [-v]");
    println!("  openhuman text-input ghost     <text> [--ttl <ms>] [-v]");
    println!("  openhuman text-input dismiss   [-v]");
    println!();
    println!("Subcommands:");
    println!("  run       Start a lightweight JSON-RPC server with text input methods");
    println!("  read      Read the currently focused text input field (JSON to stdout)");
    println!("  insert    Insert text into the focused field");
    println!("  ghost     Show ghost text overlay near the focused field");
    println!("  dismiss   Dismiss the ghost text overlay");
    println!();
    println!("Common options:");
    println!("  --port <u16>     Server port for 'run' (default: 7798)");
    println!("  --bounds         Include element bounds in 'read' output");
    println!("  --ttl <ms>       Ghost text auto-dismiss (default: 3000)");
    println!("  -v, --verbose    Enable debug logging");
}
