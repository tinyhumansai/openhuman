//! `openhuman tree-summarizer` — CLI for the hierarchical summary tree.
//!
//! Ingest content, run summarization jobs, query the tree, and inspect
//! status from the terminal without starting the full app.
//!
//! Usage:
//!   openhuman tree-summarizer ingest  <namespace> [--content <text> | --file <path>] [-v]
//!   openhuman tree-summarizer run     <namespace> [-v]
//!   openhuman tree-summarizer query   <namespace> [<node_id>] [-v]
//!   openhuman tree-summarizer status  <namespace> [-v]
//!   openhuman tree-summarizer rebuild <namespace> [-v]

use anyhow::Result;

/// Entry point for `openhuman tree-summarizer <subcommand>`.
pub fn run_tree_summarizer_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "ingest" => run_ingest(&args[1..]),
        "run" => run_summarize(&args[1..]),
        "query" => run_query(&args[1..]),
        "status" => run_status(&args[1..]),
        "rebuild" => run_rebuild(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown tree-summarizer subcommand '{other}'. Run `openhuman tree-summarizer --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Option parsing
// ---------------------------------------------------------------------------

struct CliOpts {
    verbose: bool,
    content: Option<String>,
    file: Option<String>,
    node_id: Option<String>,
}

fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut verbose = false;
    let mut content: Option<String> = None;
    let mut file: Option<String> = None;
    let mut node_id: Option<String> = None;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--content" | "-c" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --content"))?;
                content = Some(val.clone());
                i += 2;
            }
            "--file" | "-f" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --file"))?;
                file = Some(val.clone());
                i += 2;
            }
            "--node-id" | "--node" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --node-id"))?;
                node_id = Some(val.clone());
                i += 2;
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
            verbose,
            content,
            file,
            node_id,
        },
        rest,
    ))
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// `openhuman tree-summarizer ingest <namespace> --content <text>` or `--file <path>`
fn run_ingest(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman tree-summarizer ingest <namespace> [--content <text>] [--file <path>] [-v]");
        println!();
        println!("Append content to the summarization buffer for a namespace.");
        println!();
        println!("  <namespace>          Target namespace for the summary tree");
        println!("  --content, -c <text> Raw text content to ingest");
        println!("  --file, -f <path>    Read content from a file (use - for stdin)");
        println!("  -v, --verbose        Enable debug logging");
        println!();
        println!("Either --content or --file is required. If both are given, --file wins.");
        return Ok(());
    }

    let namespace = &rest[0];

    let content = if let Some(ref path) = opts.file {
        if path == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow::anyhow!("failed to read stdin: {e}"))?;
            buf
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", path))?
        }
    } else if let Some(ref text) = opts.content {
        text.clone()
    } else {
        return Err(anyhow::anyhow!(
            "either --content or --file is required. Run `openhuman tree-summarizer ingest --help`."
        ));
    };

    if content.trim().is_empty() {
        return Err(anyhow::anyhow!("content is empty"));
    }

    init_logging(opts.verbose);

    let rt = build_runtime()?;
    rt.block_on(async {
        let config = load_config().await?;
        let outcome = crate::openhuman::tree_summarizer::rpc::tree_summarizer_ingest(
            &config, namespace, &content, None, None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

/// `openhuman tree-summarizer run <namespace>`
fn run_summarize(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman tree-summarizer run <namespace> [-v]");
        println!();
        println!("Trigger the summarization job for a namespace.");
        println!("Drains the buffer, creates the hour leaf, and propagates upward.");
        println!();
        println!("  <namespace>      Target namespace");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let namespace = &rest[0];
    init_logging(opts.verbose);

    let rt = build_runtime()?;
    rt.block_on(async {
        let config = load_config().await?;
        let outcome =
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_run(&config, namespace)
                .await
                .map_err(anyhow::Error::msg)?;

        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

/// `openhuman tree-summarizer query <namespace> [<node_id>]`
fn run_query(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!(
            "Usage: openhuman tree-summarizer query <namespace> [<node_id>] [--node-id <id>] [-v]"
        );
        println!();
        println!("Read a summary tree node and its direct children.");
        println!();
        println!("  <namespace>          Target namespace");
        println!("  <node_id>            Node ID to query (default: root)");
        println!("  --node-id, --node    Alternative way to specify the node ID");
        println!("  -v, --verbose        Enable debug logging");
        println!();
        println!("Node ID examples:");
        println!("  root              All-time summary");
        println!("  2024              Year summary");
        println!("  2024/03           Month summary");
        println!("  2024/03/15        Day summary");
        println!("  2024/03/15/14     Hour leaf (2pm)");
        return Ok(());
    }

    let namespace = &rest[0];
    let node_id = opts
        .node_id
        .as_deref()
        .or_else(|| rest.get(1).map(|s| s.as_str()));

    init_logging(opts.verbose);

    let rt = build_runtime()?;
    rt.block_on(async {
        let config = load_config().await?;
        let outcome = crate::openhuman::tree_summarizer::rpc::tree_summarizer_query(
            &config, namespace, node_id,
        )
        .await
        .map_err(anyhow::Error::msg)?;

        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

/// `openhuman tree-summarizer status <namespace>`
fn run_status(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman tree-summarizer status <namespace> [-v]");
        println!();
        println!("Show tree metadata: node count, depth, date range.");
        println!();
        println!("  <namespace>      Target namespace");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let namespace = &rest[0];
    init_logging(opts.verbose);

    let rt = build_runtime()?;
    rt.block_on(async {
        let config = load_config().await?;
        let outcome =
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_status(&config, namespace)
                .await
                .map_err(anyhow::Error::msg)?;

        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

/// `openhuman tree-summarizer rebuild <namespace>`
fn run_rebuild(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) || rest.is_empty() {
        println!("Usage: openhuman tree-summarizer rebuild <namespace> [-v]");
        println!();
        println!("Rebuild the entire summary tree from hour leaves upward.");
        println!("This re-summarizes all intermediate levels (day, month, year, root).");
        println!();
        println!("  <namespace>      Target namespace");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let namespace = &rest[0];
    init_logging(opts.verbose);

    eprintln!("  Rebuilding tree for namespace '{namespace}'... this may take a while.");

    let rt = build_runtime()?;
    rt.block_on(async {
        let config = load_config().await?;
        let outcome =
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_rebuild(&config, namespace)
                .await
                .map_err(anyhow::Error::msg)?;

        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.value)
                .unwrap_or_else(|_| format!("{:?}", outcome.value))
        );
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build tokio runtime: {e}"))
}

async fn load_config() -> Result<crate::openhuman::config::Config> {
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .unwrap_or_default();
    config.apply_env_overrides();
    Ok(config)
}

fn init_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        unsafe { std::env::set_var("RUST_LOG", "warn") };
    }
    crate::core::logging::init_for_cli_run(verbose, crate::core::logging::CliLogDefault::Global);
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_help() {
    println!("openhuman tree-summarizer — hierarchical summary tree\n");
    println!("Usage:");
    println!(
        "  openhuman tree-summarizer ingest  <namespace> [--content <text>] [--file <path>] [-v]"
    );
    println!("  openhuman tree-summarizer run     <namespace> [-v]");
    println!("  openhuman tree-summarizer query   <namespace> [<node_id>] [-v]");
    println!("  openhuman tree-summarizer status  <namespace> [-v]");
    println!("  openhuman tree-summarizer rebuild <namespace> [-v]");
    println!();
    println!("Subcommands:");
    println!("  ingest    Buffer raw content for the next summarization run");
    println!("  run       Drain buffer → create hour leaf → propagate summaries upward");
    println!("  query     Read a node and its children (default: root)");
    println!("  status    Show tree metadata (node count, depth, date range)");
    println!("  rebuild   Rebuild entire tree from hour leaves (re-summarizes all levels)");
    println!();
    println!("Common options:");
    println!("  -v, --verbose    Enable debug logging");
    println!();
    println!("Examples:");
    println!("  openhuman tree-summarizer ingest my-ns --content 'Some raw data to summarize'");
    println!("  openhuman tree-summarizer ingest my-ns --file notes.txt");
    println!("  cat journal.md | openhuman tree-summarizer ingest my-ns --file -");
    println!("  openhuman tree-summarizer run my-ns");
    println!("  openhuman tree-summarizer query my-ns root");
    println!("  openhuman tree-summarizer query my-ns 2024/03/15");
    println!("  openhuman tree-summarizer status my-ns");
}
