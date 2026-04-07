//! `openhuman memory` — CLI for memory ingestion, graph inspection, and debugging.
//!
//! Provides direct access to the memory system from the command line, including
//! document ingestion with GLiNER entity/relation extraction, graph querying,
//! and document listing.
//!
//! Usage:
//!   openhuman memory ingest  <file|->  [--namespace <ns>] [--key <key>] [--title <title>] [-v]
//!   openhuman memory docs    [--namespace <ns>]
//!   openhuman memory graph   [--namespace <ns>] [--subject <s>] [--predicate <p>]
//!   openhuman memory query   --namespace <ns> --query <text> [--limit <n>]
//!   openhuman memory namespaces

use anyhow::Result;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::memory::ingestion::{MemoryIngestionConfig, MemoryIngestionRequest};
use crate::openhuman::memory::store::types::NamespaceDocumentInput;
use crate::openhuman::memory::store::unified::UnifiedMemory;
use crate::openhuman::memory::{embeddings, MemoryClient};

/// Entry point for `openhuman memory <subcommand>`.
pub fn run_memory_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_memory_help();
        return Ok(());
    }

    match args[0].as_str() {
        "ingest" => run_ingest(&args[1..]),
        "docs" | "list" => run_docs(&args[1..]),
        "graph" | "graph-query" => run_graph_query(&args[1..]),
        "query" => run_query(&args[1..]),
        "namespaces" | "ns" => run_namespaces(&args[1..]),
        "clear" => run_clear(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown memory subcommand '{other}'. Run `openhuman memory --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// `openhuman memory ingest <file|-> [options]`
///
/// Reads a file (or stdin with `-`) and performs full synchronous ingestion
/// including GLiNER entity/relation extraction. Outputs the ingestion result
/// as JSON for debugging.
fn run_ingest(args: &[String]) -> Result<()> {
    let mut file_path: Option<String> = None;
    let mut namespace = "cli".to_string();
    let mut key: Option<String> = None;
    let mut title: Option<String> = None;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--namespace" | "-n" => {
                namespace = next_arg(args, &mut i, "--namespace")?;
            }
            "--key" | "-k" => {
                key = Some(next_arg(args, &mut i, "--key")?);
            }
            "--title" | "-t" => {
                title = Some(next_arg(args, &mut i, "--title")?);
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman memory ingest <file|-> [options]");
                println!();
                println!("  <file>               Path to file to ingest (use '-' for stdin)");
                println!("  -n, --namespace <ns>  Target namespace (default: 'cli')");
                println!("  -k, --key <key>       Document key for dedup (default: filename)");
                println!("  -t, --title <title>   Document title (default: filename)");
                println!("  -v, --verbose         Enable debug logging");
                return Ok(());
            }
            other if !other.starts_with('-') && file_path.is_none() => {
                file_path = Some(other.to_string());
                i += 1;
            }
            other => return Err(anyhow::anyhow!("unknown ingest arg: {other}")),
        }
    }

    let file_path = file_path.ok_or_else(|| {
        anyhow::anyhow!("missing file argument. Use a file path or '-' for stdin.")
    })?;

    crate::core::logging::init_for_cli_run(verbose);

    let content = read_input(&file_path)?;
    let doc_key = key.unwrap_or_else(|| file_path.clone());
    let doc_title = title.unwrap_or_else(|| {
        if file_path == "-" {
            "stdin-input".to_string()
        } else {
            PathBuf::from(&file_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone())
        }
    });

    eprintln!(
        "[memory:cli] ingesting document: namespace={namespace}, key={doc_key}, title={doc_title}, \
         content_len={}",
        content.len()
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .unwrap_or_default();
        let workspace_dir = config.workspace_dir;

        std::fs::create_dir_all(&workspace_dir)
            .map_err(|e| format!("create workspace dir: {e}"))?;

        let embedder: Arc<dyn crate::openhuman::memory::embeddings::EmbeddingProvider> =
            embeddings::default_local_embedding_provider();
        let memory =
            UnifiedMemory::new(&workspace_dir, embedder, None).map_err(|e| format!("{e}"))?;

        let document = NamespaceDocumentInput {
            namespace: namespace.clone(),
            key: doc_key,
            title: doc_title,
            content,
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: serde_json::json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        };

        let ingestion_config = MemoryIngestionConfig::default();

        eprintln!(
            "[memory:cli] starting ingestion with model={}, extraction_mode={}",
            ingestion_config.model_name,
            ingestion_config.extraction_mode.as_str()
        );

        let result = memory
            .ingest_document(MemoryIngestionRequest {
                document,
                config: ingestion_config,
            })
            .await?;

        Ok::<_, String>(result)
    })?;

    eprintln!();
    eprintln!("=== Ingestion Result ===");
    eprintln!("  document_id:  {}", result.document_id);
    eprintln!("  namespace:    {}", result.namespace);
    eprintln!("  model:        {}", result.model_name);
    eprintln!("  mode:         {}", result.extraction_mode);
    eprintln!("  chunks:       {}", result.chunk_count);
    eprintln!("  entities:     {}", result.entity_count);
    eprintln!("  relations:    {}", result.relation_count);
    eprintln!("  preferences:  {}", result.preference_count);
    eprintln!("  decisions:    {}", result.decision_count);
    eprintln!("  tags:         {:?}", result.tags);

    // Print full JSON to stdout for piping/scripting
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

/// `openhuman memory docs [--namespace <ns>]`
fn run_docs(args: &[String]) -> Result<()> {
    let mut namespace: Option<String> = None;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--namespace" | "-n" => {
                namespace = Some(next_arg(args, &mut i, "--namespace")?);
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman memory docs [--namespace <ns>] [-v]");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown docs arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let client = create_memory_client().await?;
        client
            .list_documents(namespace.as_deref())
            .await
            .map_err(anyhow::Error::msg)
    })?;

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// `openhuman memory graph [--namespace <ns>] [--subject <s>] [--predicate <p>]`
fn run_graph_query(args: &[String]) -> Result<()> {
    let mut namespace: Option<String> = None;
    let mut subject: Option<String> = None;
    let mut predicate: Option<String> = None;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--namespace" | "-n" => {
                namespace = Some(next_arg(args, &mut i, "--namespace")?);
            }
            "--subject" | "-s" => {
                subject = Some(next_arg(args, &mut i, "--subject")?);
            }
            "--predicate" | "-p" => {
                predicate = Some(next_arg(args, &mut i, "--predicate")?);
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: openhuman memory graph [--namespace <ns>] [--subject <s>] [--predicate <p>] [-v]"
                );
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown graph arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let client = create_memory_client().await?;
        client
            .graph_query(namespace.as_deref(), subject.as_deref(), predicate.as_deref())
            .await
            .map_err(anyhow::Error::msg)
    })?;

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// `openhuman memory query --namespace <ns> --query <text> [--limit <n>]`
fn run_query(args: &[String]) -> Result<()> {
    let mut namespace: Option<String> = None;
    let mut query: Option<String> = None;
    let mut limit: usize = 10;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--namespace" | "-n" => {
                namespace = Some(next_arg(args, &mut i, "--namespace")?);
            }
            "--query" | "-q" => {
                query = Some(next_arg(args, &mut i, "--query")?);
            }
            "--limit" | "-l" => {
                let raw = next_arg(args, &mut i, "--limit")?;
                limit = raw.parse().map_err(|e| anyhow::anyhow!("invalid --limit: {e}"))?;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: openhuman memory query --namespace <ns> --query <text> [--limit <n>] [-v]"
                );
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown query arg: {other}")),
        }
    }

    let namespace =
        namespace.ok_or_else(|| anyhow::anyhow!("--namespace is required for query"))?;
    let query = query.ok_or_else(|| anyhow::anyhow!("--query is required"))?;

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let client = create_memory_client().await?;
        client
            .query_namespace(&namespace, &query, limit)
            .await
            .map_err(anyhow::Error::msg)
    })?;

    println!("{result}");
    Ok(())
}

/// `openhuman memory namespaces`
fn run_namespaces(args: &[String]) -> Result<()> {
    let mut verbose = false;
    for arg in args {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => {
                println!("Usage: openhuman memory namespaces [-v]");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown namespaces arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let client = create_memory_client().await?;
        client.list_namespaces().await.map_err(anyhow::Error::msg)
    })?;

    for ns in &result {
        println!("{ns}");
    }
    Ok(())
}

/// `openhuman memory clear --namespace <ns>`
fn run_clear(args: &[String]) -> Result<()> {
    let mut namespace: Option<String> = None;
    let mut verbose = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--namespace" | "-n" => {
                namespace = Some(next_arg(args, &mut i, "--namespace")?);
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman memory clear --namespace <ns> [-v]");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown clear arg: {other}")),
        }
    }

    let namespace =
        namespace.ok_or_else(|| anyhow::anyhow!("--namespace is required for clear"))?;

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let client = create_memory_client().await?;
        client
            .clear_namespace(&namespace)
            .await
            .map_err(anyhow::Error::msg)?;
        eprintln!("[memory:cli] namespace '{namespace}' cleared");
        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_help(s: &str) -> bool {
    matches!(s, "-h" | "--help" | "help")
}

fn next_arg(args: &[String], i: &mut usize, flag: &str) -> Result<String> {
    let value = args
        .get(*i + 1)
        .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?
        .clone();
    *i += 2;
    Ok(value)
}

fn read_input(path: &str) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        let path = PathBuf::from(path);
        if !path.exists() {
            return Err(anyhow::anyhow!("file not found: {}", path.display()));
        }
        Ok(std::fs::read_to_string(&path)?)
    }
}

async fn create_memory_client() -> Result<MemoryClient> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .unwrap_or_default();
    MemoryClient::from_workspace_dir(config.workspace_dir).map_err(anyhow::Error::msg)
}

fn print_memory_help() {
    println!("Usage: openhuman memory <subcommand> [options]");
    println!();
    println!("Subcommands:");
    println!("  ingest <file|->     Ingest a document with full GLiNER extraction");
    println!("  docs                List stored documents");
    println!("  graph               Query the knowledge graph");
    println!("  query               Semantic query against a namespace");
    println!("  namespaces          List all namespaces");
    println!("  clear               Clear all data in a namespace");
    println!();
    println!("Examples:");
    println!("  openhuman memory ingest notes.md -n my-project -v");
    println!("  echo 'Alice works on ProjectX' | openhuman memory ingest - -n test -v");
    println!("  openhuman memory graph -n my-project");
    println!("  openhuman memory docs -n my-project");
    println!("  openhuman memory query -n my-project -q 'who works on what?'");
}
