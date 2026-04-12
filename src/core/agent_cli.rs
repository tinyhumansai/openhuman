//! `openhuman agent` — developer CLI for inspecting agent definitions and
//! the system prompts the context engine produces for them.
//!
//! This is intentionally scoped to *debugging*: no execution, no provider
//! calls, no server boot. Every subcommand boils down to reading config /
//! agent definitions / tool registry and printing something.
//!
//! Usage:
//!   openhuman agent dump-prompt --agent <id> [--skill <id>] [--workspace <path>] [--json] [--with-tools] [-v]
//!   openhuman agent list [--json] [-v]
//!
//! `dump-prompt` is the main tool: it renders the exact system prompt the
//! context engine would hand to the LLM when that agent is spawned. Pass
//! `--agent main` for the orchestrator / main-agent prompt; otherwise pass
//! any built-in or workspace-custom agent id (e.g. `skills_agent`,
//! `orchestrator`, `code_executor`).
//!
//! `--skill` mirrors the `spawn_subagent { skill_filter: "…" }` runtime
//! argument — pair it with `skills_agent` to scope the dump to a single
//! integration (e.g. `--skill notion`).

use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::context::debug_dump::{dump_agent_prompt, DumpPromptOptions, DumpedPrompt};

/// Entry point for `openhuman agent <subcommand>`.
pub fn run_agent_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_agent_help();
        return Ok(());
    }

    match args[0].as_str() {
        "dump-prompt" => run_dump_prompt(&args[1..]),
        "list" => run_list(&args[1..]),
        other => Err(anyhow!(
            "unknown agent subcommand '{other}'. Run `openhuman agent --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// dump-prompt
// ---------------------------------------------------------------------------

struct DumpFlags {
    agent: Option<String>,
    skill: Option<String>,
    workspace: Option<PathBuf>,
    model: Option<String>,
    json: bool,
    with_tools: bool,
    stub_composio: bool,
    verbose: bool,
}

fn parse_dump_flags(args: &[String]) -> Result<DumpFlags> {
    let mut out = DumpFlags {
        agent: None,
        skill: None,
        workspace: None,
        model: None,
        json: false,
        with_tools: false,
        stub_composio: false,
        verbose: false,
    };
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--agent" | "-a" => {
                out.agent = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --agent"))?
                        .clone(),
                );
                i += 2;
            }
            "--skill" | "-s" => {
                out.skill = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --skill"))?
                        .clone(),
                );
                i += 2;
            }
            "--workspace" | "-w" => {
                out.workspace = Some(PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --workspace"))?,
                ));
                i += 2;
            }
            "--model" | "-m" => {
                out.model = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --model"))?
                        .clone(),
                );
                i += 2;
            }
            "--json" => {
                out.json = true;
                i += 1;
            }
            "--with-tools" => {
                out.with_tools = true;
                i += 1;
            }
            "--stub-composio" => {
                out.stub_composio = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                out.verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_dump_prompt_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown dump-prompt arg: {other}")),
        }
        let _ = i; // silence unused-warning in the `help` branch
    }
    Ok(out)
}

fn run_dump_prompt(args: &[String]) -> Result<()> {
    let flags = parse_dump_flags(args)?;
    let agent = flags
        .agent
        .clone()
        .ok_or_else(|| anyhow!("--agent <id> is required (use `main` for the orchestrator)"))?;

    init_quiet_logging(flags.verbose);

    let options = DumpPromptOptions {
        agent_id: agent,
        skill_filter: flags.skill.clone(),
        workspace_dir_override: flags.workspace.clone(),
        model_override: flags.model.clone(),
        stub_composio: flags.stub_composio,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let dumped = rt.block_on(async { dump_agent_prompt(options).await })?;

    if flags.json {
        print_json(&dumped, flags.with_tools)?;
    } else {
        print_human(&dumped, flags.with_tools);
    }
    Ok(())
}

fn print_human(dumped: &DumpedPrompt, with_tools: bool) {
    // Banner on stderr so `openhuman agent dump-prompt ... > file.md` stays
    // clean — stdout is the prompt, stderr is the metadata. This matches
    // the pattern already used by `run_call_command` / `run_server_command`
    // in `core/cli.rs` (banner to stderr, JSON result to stdout).
    eprintln!("# Agent prompt dump");
    eprintln!("agent:          {}", dumped.agent_id);
    eprintln!("mode:           {}", dumped.mode);
    eprintln!("model:          {}", dumped.model);
    eprintln!("workspace:      {}", dumped.workspace_dir.display());
    eprintln!("tool_count:     {}", dumped.tool_names.len());
    eprintln!("skill_tools:    {}", dumped.skill_tool_count);
    match dumped.cache_boundary {
        Some(offset) => eprintln!("cache_boundary: byte offset {offset}"),
        None => eprintln!("cache_boundary: none"),
    }
    if with_tools {
        eprintln!("tools:");
        for name in &dumped.tool_names {
            eprintln!("  - {name}");
        }
    }
    eprintln!();
    eprintln!("─── BEGIN SYSTEM PROMPT ───");
    println!("{}", dumped.text);
    eprintln!("─── END SYSTEM PROMPT ───");
}

fn print_json(dumped: &DumpedPrompt, with_tools: bool) -> Result<()> {
    // Use a plain serde_json::Value so we don't need to add Serialize to
    // DumpedPrompt (which would pull the agent harness types into our
    // serde surface). This output is stable and scriptable from bash.
    let mut obj = serde_json::Map::new();
    obj.insert(
        "agent_id".into(),
        serde_json::Value::String(dumped.agent_id.clone()),
    );
    obj.insert(
        "mode".into(),
        serde_json::Value::String(dumped.mode.to_string()),
    );
    obj.insert(
        "model".into(),
        serde_json::Value::String(dumped.model.clone()),
    );
    obj.insert(
        "workspace_dir".into(),
        serde_json::Value::String(dumped.workspace_dir.display().to_string()),
    );
    obj.insert(
        "tool_count".into(),
        serde_json::Value::Number(dumped.tool_names.len().into()),
    );
    obj.insert(
        "skill_tool_count".into(),
        serde_json::Value::Number(dumped.skill_tool_count.into()),
    );
    obj.insert(
        "cache_boundary".into(),
        match dumped.cache_boundary {
            Some(offset) => serde_json::Value::Number(offset.into()),
            None => serde_json::Value::Null,
        },
    );
    obj.insert(
        "system_prompt".into(),
        serde_json::Value::String(dumped.text.clone()),
    );
    if with_tools {
        obj.insert(
            "tools".into(),
            serde_json::Value::Array(
                dumped
                    .tool_names
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(obj))?
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

fn run_list(args: &[String]) -> Result<()> {
    let mut as_json = false;
    let mut workspace: Option<PathBuf> = None;
    let mut verbose = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                as_json = true;
                i += 1;
            }
            "--workspace" | "-w" => {
                workspace = Some(PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --workspace"))?,
                ));
                i += 2;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman agent list [--workspace <path>] [--json] [-v]");
                println!();
                println!("  List every built-in agent plus any custom `<workspace>/agents/*.toml` overrides.");
                return Ok(());
            }
            other => return Err(anyhow!("unknown list arg: {other}")),
        }
    }

    // Silence the logger so Config::load_or_init and AgentDefinitionRegistry::load
    // don't write warnings/info to stdout, which would corrupt --json output.
    // (The project's CLI logger writes to stdout, not stderr.)
    init_quiet_logging(verbose);

    // Resolve a workspace directory so workspace-custom overrides are
    // picked up. Fall back to the config's default when no --workspace is
    // passed, matching the lookup behaviour the runtime uses at spawn time.
    let resolved_workspace = if let Some(ws) = workspace {
        ws
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let config =
            rt.block_on(async { crate::openhuman::config::Config::load_or_init().await })?;
        config.workspace_dir
    };

    let registry = AgentDefinitionRegistry::load(&resolved_workspace)?;
    if as_json {
        let mut arr = Vec::new();
        for def in registry.list() {
            let mut obj = serde_json::Map::new();
            obj.insert("id".into(), serde_json::Value::String(def.id.clone()));
            obj.insert(
                "display_name".into(),
                serde_json::Value::String(def.display_name().to_string()),
            );
            obj.insert(
                "when_to_use".into(),
                serde_json::Value::String(def.when_to_use.clone()),
            );
            obj.insert(
                "omit_safety_preamble".into(),
                serde_json::Value::Bool(def.omit_safety_preamble),
            );
            obj.insert(
                "omit_identity".into(),
                serde_json::Value::Bool(def.omit_identity),
            );
            obj.insert(
                "omit_skills_catalog".into(),
                serde_json::Value::Bool(def.omit_skills_catalog),
            );
            obj.insert(
                "category_filter".into(),
                match def.category_filter {
                    Some(cat) => serde_json::Value::String(format!("{cat:?}")),
                    None => serde_json::Value::Null,
                },
            );
            arr.push(serde_json::Value::Object(obj));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(arr))?
        );
    } else {
        println!("{:<20} {:<22} WHEN TO USE", "ID", "CATEGORY FILTER");
        println!("{}", "-".repeat(90));
        for def in registry.list() {
            let cat = def
                .category_filter
                .map(|c| format!("{c:?}"))
                .unwrap_or_else(|| "-".into());
            let when = def.when_to_use.chars().take(46).collect::<String>();
            println!("{:<20} {:<22} {}", def.id, cat, when);
        }
        println!();
        println!("{} agent(s) registered.", registry.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

fn print_agent_help() {
    println!("openhuman agent — inspect agents and the prompts they receive");
    println!();
    println!("Usage:");
    println!("  openhuman agent list [--workspace <path>] [--json]");
    println!("  openhuman agent dump-prompt --agent <id> [--skill <id>] [--workspace <path>] [--model <name>] [--with-tools] [--json] [-v]");
    println!();
    println!("Run `openhuman agent <subcommand> --help` for details.");
}

fn print_dump_prompt_help() {
    println!("openhuman agent dump-prompt — render the exact system prompt an agent receives");
    println!();
    println!("Usage:");
    println!("  openhuman agent dump-prompt --agent <id> [options]");
    println!();
    println!("Required:");
    println!("  --agent, -a <id>     Target agent id — any built-in or workspace-custom id.");
    println!("                       Use `main` for the orchestrator / main-agent prompt.");
    println!();
    println!("Options:");
    println!("  --skill, -s <id>     Skill filter override — scopes the tool list to");
    println!("                       tools named `<id>__*`. Mirrors `spawn_subagent`'s");
    println!("                       per-call `skill_filter` argument.");
    println!("  --workspace, -w <p>  Override the workspace directory (defaults to");
    println!("                       Config::workspace_dir / ~/.openhuman/workspace).");
    println!("  --model, -m <name>   Override the resolved model name (affects only the");
    println!("                       `## Runtime` section).");
    println!("  --with-tools         Also print the full list of tool names the agent sees.");
    println!("  --stub-composio      Inject the five Composio meta-tool stubs into the dump");
    println!("                       even if the user is not signed in. Debug-only — do not");
    println!("                       run agents against the resulting registry (stubs have no");
    println!("                       real backend).");
    println!("  --json               Emit a machine-readable JSON object on stdout.");
    println!("  -v, --verbose        Enable debug logging on stderr.");
    println!();
    println!("Examples:");
    println!("  # Full skills_agent dump (includes Composio meta-tools when enabled).");
    println!("  openhuman agent dump-prompt --agent skills_agent --with-tools");
    println!();
    println!("  # Narrow the skills_agent prompt to just the Notion integration.");
    println!("  openhuman agent dump-prompt --agent skills_agent --skill notion");
    println!();
    println!("  # Main orchestrator prompt, JSON for scripting.");
    println!("  openhuman agent dump-prompt --agent main --json");
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

/// Quiet logging: only `error` unless verbose. We pin this lower than
/// `warn` (the default in `skills_cli::init_quiet_logging`) because
/// `agent dump-prompt` is designed to be redirected into a file, and
/// expected warnings like `[integrations] no auth token available …`
/// would otherwise interleave with the rendered prompt body on stdout
/// (the project's CLI logger writes to stdout, not stderr). Verbose
/// users can opt back in with `-v` or `RUST_LOG=…`.
fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "error");
    }
    crate::core::logging::init_for_cli_run(verbose, crate::core::logging::CliLogDefault::Global);
}
