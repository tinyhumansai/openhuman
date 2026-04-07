//! `openhuman skills` — lightweight CLI for skill development.
//!
//! Boots **only** the QuickJS skill runtime (no Socket.IO, no local-AI, no
//! Telegram auth) and exposes a minimal JSON-RPC server so skill authors can
//! test against the real runtime without building the full desktop app.
//!
//! Usage:
//!   openhuman skills run   [--skills-dir <path>] [--port <u16>] [-v]
//!   openhuman skills list  [--skills-dir <path>]
//!   openhuman skills start <skill-id> [--skills-dir <path>]
//!   openhuman skills call  <skill-id> <tool-name> [--args '<json>'] [--skills-dir <path>]
//!   openhuman skills test  <skill-id> [--skills-dir <path>]

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// Entry point for `openhuman skills <subcommand>`.
pub fn run_skills_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_skills_help();
        return Ok(());
    }

    match args[0].as_str() {
        "run" => run_skills_server(&args[1..]),
        "list" => run_skills_list(&args[1..]),
        "start" => run_skills_start(&args[1..]),
        "call" => run_skills_call(&args[1..]),
        "test" => run_skills_test(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown skills subcommand '{other}'. Run `openhuman skills --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Common options parsed from CLI flags.
struct SkillsOpts {
    skills_dir: Option<PathBuf>,
    port: u16,
    verbose: bool,
}

fn parse_common_opts(args: &[String]) -> Result<(SkillsOpts, Vec<String>)> {
    let mut skills_dir: Option<PathBuf> = None;
    let mut port: u16 = 7799; // default skills-dev port (different from main 7788)
    let mut verbose = false;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--skills-dir" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --skills-dir"))?;
                skills_dir = Some(PathBuf::from(val));
                i += 2;
            }
            "--port" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --port: {e}"))?;
                i += 2;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                // Caller will handle this
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
        SkillsOpts {
            skills_dir,
            port,
            verbose,
        },
        rest,
    ))
}

/// Bootstrap the QuickJS skill runtime with an optional skills directory override.
async fn bootstrap_skills_runtime(
    skills_dir: Option<&PathBuf>,
) -> Result<Arc<crate::openhuman::skills::qjs_engine::RuntimeEngine>> {
    use crate::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};

    // If --skills-dir is given, set SKILLS_LOCAL_DIR so the engine picks it up.
    if let Some(dir) = skills_dir {
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        std::env::set_var("SKILLS_LOCAL_DIR", &canonical);
        log::info!("[skills-cli] SKILLS_LOCAL_DIR = {:?}", canonical);
    }

    // Resolve the base directory (~/.openhuman or $OPENHUMAN_WORKSPACE).
    let base_dir = std::env::var("OPENHUMAN_WORKSPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".openhuman")
        });

    let skills_data_dir = base_dir.join("skills_data");
    std::fs::create_dir_all(&skills_data_dir)
        .map_err(|e| anyhow::anyhow!("failed to create skills data dir: {e}"))?;

    let engine = RuntimeEngine::new(skills_data_dir)
        .map_err(|e| anyhow::anyhow!("failed to create RuntimeEngine: {e}"))?;
    let engine = Arc::new(engine);

    // Point at workspace directory for user-installed skills.
    let workspace_dir = base_dir.join("workspace");
    let _ = std::fs::create_dir_all(&workspace_dir);
    engine.set_workspace_dir(workspace_dir);

    // Register globally so RPC handlers can access it.
    set_global_engine(engine.clone());

    // Start cron + ping schedulers.
    engine.ping_scheduler().start();
    engine.cron_scheduler().start();

    log::info!("[skills-cli] Skill runtime initialized");
    Ok(engine)
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// `openhuman skills run` — start a minimal JSON-RPC server with just the skill runtime.
fn run_skills_server(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_common_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman skills run [--skills-dir <path>] [--port <u16>] [-v]");
        println!();
        println!("Start a lightweight JSON-RPC server with only the skill runtime.");
        println!("Skills are auto-discovered and auto-started.");
        println!();
        println!(
            "  --skills-dir <path>  Directory containing compiled skills (default: auto-detect)"
        );
        println!("  --port <u16>         Listen port (default: 7799)");
        println!("  -v, --verbose        Enable debug logging");
        return Ok(());
    }

    crate::core::logging::init_for_cli_run(
        opts.verbose,
        crate::core::logging::CliLogDefault::Global,
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_skills_runtime(opts.skills_dir.as_ref()).await?;

        // Auto-start all skills.
        engine.auto_start_skills().await;

        // Build a minimal HTTP router (health + JSON-RPC only).
        let app = build_skills_only_router();

        let bind_addr = format!("127.0.0.1:{}", opts.port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

        log::info!("[skills-cli] Skills runtime ready — http://{bind_addr}/rpc (JSON-RPC 2.0)");
        log::info!("[skills-cli] Health check: http://{bind_addr}/health");

        // Print discovered skills summary.
        let skills = engine.list_skills();
        if skills.is_empty() {
            log::warn!(
                "[skills-cli] No skills discovered. Check --skills-dir or SKILLS_LOCAL_DIR."
            );
        } else {
            for snap in &skills {
                log::info!("[skills-cli]   {} — {:?}", snap.skill_id, snap.status,);
            }
        }

        eprintln!();
        eprintln!("  Skills dev server listening on http://{bind_addr}");
        eprintln!("  Press Ctrl+C to stop.");
        eprintln!();

        axum::serve(listener, app).await?;
        Ok(())
    })
}

/// `openhuman skills list` — discover and list available skills.
fn run_skills_list(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_common_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman skills list [--skills-dir <path>]");
        println!();
        println!("Discover and list all available skills.");
        return Ok(());
    }

    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_skills_runtime(opts.skills_dir.as_ref()).await?;
        let manifests = engine.discover_skills().await.map_err(anyhow::Error::msg)?;

        if manifests.is_empty() {
            println!("No skills found.");
        } else {
            println!("{:<20} {:<10} {}", "ID", "VERSION", "NAME");
            println!("{}", "-".repeat(60));
            for m in &manifests {
                println!(
                    "{:<20} {:<10} {}",
                    m.id,
                    m.version.as_deref().unwrap_or("-"),
                    m.name
                );
            }
            println!("\n{} skill(s) found.", manifests.len());
        }
        Ok(())
    })
}

/// `openhuman skills start <skill-id>` — start a single skill and print its state.
fn run_skills_start(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_common_opts(args)?;

    if rest.is_empty() || rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman skills start <skill-id> [--skills-dir <path>] [-v]");
        println!();
        println!("Start a single skill and print its initialization state.");
        return Ok(());
    }

    let skill_id = &rest[0];
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_skills_runtime(opts.skills_dir.as_ref()).await?;

        match engine.start_skill(skill_id).await {
            Ok(snap) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snap).unwrap_or_else(|_| format!("{:?}", snap))
                );
            }
            Err(e) => {
                eprintln!("Failed to start skill '{}': {}", skill_id, e);
                std::process::exit(1);
            }
        }
        Ok(())
    })
}

/// `openhuman skills call <skill-id> <tool-name> [--args '<json>']` — call a tool.
fn run_skills_call(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_common_opts(args)?;

    // Extract --args from rest
    let mut tool_args = serde_json::Value::Object(serde_json::Map::new());
    let mut positional = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--args" {
            let val = rest
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("missing value for --args"))?;
            tool_args = serde_json::from_str(val)
                .map_err(|e| anyhow::anyhow!("invalid --args JSON: {e}"))?;
            i += 2;
        } else if is_help(&rest[i]) {
            println!("Usage: openhuman skills call <skill-id> <tool-name> [--args '<json>'] [--skills-dir <path>] [-v]");
            println!();
            println!("Start a skill, then call one of its tools and print the result.");
            return Ok(());
        } else {
            positional.push(rest[i].clone());
            i += 1;
        }
    }

    if positional.len() < 2 {
        return Err(anyhow::anyhow!(
            "expected: openhuman skills call <skill-id> <tool-name>"
        ));
    }

    let skill_id = &positional[0];
    let tool_name = &positional[1];

    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_skills_runtime(opts.skills_dir.as_ref()).await?;

        // Start the skill first.
        engine
            .start_skill(skill_id)
            .await
            .map_err(anyhow::Error::msg)?;

        // Call the tool.
        match engine.call_tool(skill_id, tool_name, tool_args).await {
            Ok(result) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| format!("{:?}", result))
                );
            }
            Err(e) => {
                eprintln!("Tool call failed: {}.{} — {}", skill_id, tool_name, e);
                std::process::exit(1);
            }
        }
        Ok(())
    })
}

/// `openhuman skills test <skill-id>` — start a skill, run basic health checks.
fn run_skills_test(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_common_opts(args)?;

    if rest.is_empty() || rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman skills test <skill-id> [--skills-dir <path>] [-v]");
        println!();
        println!("Start a skill, verify lifecycle hooks, list tools, and print a summary.");
        return Ok(());
    }

    let skill_id = &rest[0];
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_skills_runtime(opts.skills_dir.as_ref()).await?;

        eprintln!("--- Testing skill: {} ---\n", skill_id);

        // 1. Start the skill.
        eprint!("  [1/3] Starting skill... ");
        match engine.start_skill(skill_id).await {
            Ok(snap) => {
                eprintln!("OK ({:?})", snap.status);
            }
            Err(e) => {
                eprintln!("FAIL: {}", e);
                std::process::exit(1);
            }
        }

        // 2. List tools.
        eprint!("  [2/3] Listing tools... ");
        let all_tools = engine.all_tools();
        let skill_tools: Vec<_> = all_tools
            .iter()
            .filter(|(sid, _)| sid == skill_id)
            .collect();
        eprintln!("{} tool(s) registered", skill_tools.len());
        for (_, tool) in &skill_tools {
            eprintln!("         - {}: {}", tool.name, tool.description);
        }

        // 3. Stop the skill.
        eprint!("  [3/3] Stopping skill... ");
        match engine.stop_skill(skill_id).await {
            Ok(()) => eprintln!("OK"),
            Err(e) => eprintln!("WARN: {}", e),
        }

        eprintln!("\n--- All checks passed ---");
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Minimal HTTP router (skills-only)
// ---------------------------------------------------------------------------

fn build_skills_only_router() -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/health", get(health))
        .route("/rpc", post(rpc))
        .route("/tools", get(list_tools))
        .route("/skills", get(list_skills))
}

async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({ "ok": true, "mode": "skills-dev" }))
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

async fn list_tools() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::skills::global_engine();
    match engine {
        Some(e) => {
            let tools: Vec<_> = e
                .all_tools()
                .into_iter()
                .map(|(skill_id, tool)| {
                    serde_json::json!({
                        "skill_id": skill_id,
                        "name": tool.name,
                        "description": tool.description,
                    })
                })
                .collect();
            axum::Json(serde_json::json!({ "tools": tools }))
        }
        None => axum::Json(serde_json::json!({ "tools": [], "error": "runtime not initialized" })),
    }
}

async fn list_skills() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::skills::global_engine();
    match engine {
        Some(e) => {
            let skills = e.list_skills();
            axum::Json(serde_json::json!({ "skills": skills }))
        }
        None => axum::Json(serde_json::json!({ "skills": [], "error": "runtime not initialized" })),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Quiet logging: only `warn` unless verbose (used for non-server subcommands).
fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::core::logging::init_for_cli_run(verbose, crate::core::logging::CliLogDefault::Global);
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_skills_help() {
    println!("openhuman skills — skill development runtime\n");
    println!("Usage:");
    println!("  openhuman skills run   [--skills-dir <path>] [--port <u16>] [-v]");
    println!("  openhuman skills list  [--skills-dir <path>]");
    println!("  openhuman skills start <skill-id> [--skills-dir <path>] [-v]");
    println!(
        "  openhuman skills call  <skill-id> <tool-name> [--args '<json>'] [--skills-dir <path>]"
    );
    println!("  openhuman skills test  <skill-id> [--skills-dir <path>] [-v]");
    println!();
    println!("Subcommands:");
    println!("  run    Start a lightweight JSON-RPC server with only the skill runtime");
    println!("  list   Discover and list available skills");
    println!("  start  Start a single skill and print its state");
    println!("  call   Start a skill and call one of its tools");
    println!("  test   Start a skill, verify lifecycle, list tools, stop");
    println!();
    println!("Common options:");
    println!("  --skills-dir <path>  Compiled skills directory (overrides auto-detection)");
    println!("  --port <u16>         Server port for 'run' (default: 7799)");
    println!("  -v, --verbose        Enable debug logging");
}
