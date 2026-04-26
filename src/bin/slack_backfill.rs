//! Manual smoke/backfill trigger for the Composio-backed Slack
//! provider.
//!
//! Invokes the same path the 15-minute periodic scheduler uses —
//! `SlackProvider::sync()` for each active Slack Composio connection —
//! but runs exactly **once** so operators can observe results end to
//! end before trusting the scheduler.
//!
//! # Prerequisites
//!
//! - A working openhuman install (same workspace dir the desktop app
//!   uses) with a signed-in session JWT.
//! - A Slack connection created via Composio's OAuth flow (e.g. from
//!   the desktop app's Integrations screen). No self-hosted Slack App
//!   or bot token is needed — authorization lives in Composio.
//! - Ollama pulled with whatever models you want the ingest pipeline to
//!   use (embedder, LLM NER, LLM summariser). Any of these can be left
//!   unconfigured — `memory/tree/ingest` soft-falls-back per call.
//!
//! # Usage
//!
//! ```sh
//! export OPENHUMAN_WORKSPACE=/path/to/workspace      # must match desktop app
//! export OPENHUMAN_MEMORY_EMBED_ENDPOINT=http://localhost:11434
//! export OPENHUMAN_MEMORY_EMBED_MODEL=nomic-embed-text
//! export OPENHUMAN_MEMORY_EXTRACT_ENDPOINT=http://localhost:11434
//! export OPENHUMAN_MEMORY_EXTRACT_MODEL=qwen2.5:0.5b
//! export OPENHUMAN_MEMORY_SUMMARISE_ENDPOINT=http://localhost:11434
//! export OPENHUMAN_MEMORY_SUMMARISE_MODEL=llama3.1:8b
//! export RUST_LOG=info,openhuman_core::openhuman::composio::providers::slack=debug,openhuman_core::openhuman::memory=debug
//!
//! cargo run --bin slack-backfill                              # all active slack connections
//! cargo run --bin slack-backfill -- --connection conn_abc     # one specific connection
//! ```

use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::Parser;

use openhuman_core::openhuman::composio::client::build_composio_client;
use openhuman_core::openhuman::composio::providers::registry::{
    get_provider, init_default_providers,
};
use openhuman_core::openhuman::composio::providers::slack::run_backfill_via_search;
use openhuman_core::openhuman::composio::providers::{ProviderContext, SyncReason};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory;

#[derive(Parser, Debug)]
#[command(
    name = "slack-backfill",
    about = "Run SlackProvider::sync() once against the user's Composio-authorized Slack connection(s)."
)]
struct Cli {
    /// Optional Composio connection id. When omitted, every active
    /// Slack connection is synced.
    #[arg(long = "connection")]
    connection_id: Option<String>,

    /// Reset the per-connection SyncState before syncing — wipes the
    /// per-channel cursor map + dedup set + daily budget. The next
    /// sync re-walks the full backfill window. Useful when you've
    /// changed canonicalisation logic and want to overwrite existing
    /// chunks (chunk-id determinism makes the rewrite an UPSERT).
    #[arg(long = "reset-state", default_value_t = false)]
    reset_state: bool,

    /// One-shot: invoke `SLACK_SEARCH_MESSAGES` with a small query and
    /// print the raw response, then exit. Probe to see if the
    /// workspace's Slack plan supports `search.messages` (paid plans
    /// only) before we consider rebuilding the provider around it.
    /// Skips the normal backfill flow.
    #[arg(long = "probe-search", default_value_t = false)]
    probe_search: bool,

    /// Use the workspace-wide `SLACK_SEARCH_MESSAGES` path instead of
    /// per-channel `conversations.history`. Better quota efficiency
    /// (each successful call returns matches across many channels)
    /// but requires the workspace to be on a paid Slack plan.
    /// `--days` controls the backfill window.
    #[arg(long = "use-search", default_value_t = false)]
    use_search: bool,

    /// Backfill window in days when `--use-search` is set. Defaults to
    /// 30 unless `OPENHUMAN_SLACK_BACKFILL_DAYS` overrides.
    #[arg(long = "days", default_value_t = 30)]
    days: i64,

    /// Synthesise a tiny single-message `ChatBatch` and ingest it
    /// under the existing per-connection `source_id` to trigger a
    /// seal cascade against the existing L0 buffer (without
    /// re-fetching from Slack/Composio). Useful after fixing a seal-
    /// downstream bug — the existing 15k-token buffer immediately
    /// re-attempts cascade on the next append.
    #[arg(long = "seal-probe", default_value_t = false)]
    seal_probe: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // env_logger captures `log::*` events (used by reqwest, the
    // memory-tree pipeline, the slack ingestion ops layer, …).
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .try_init()
        .ok(); // ignore double-init in test harness scenarios.

    // tracing-subscriber captures `tracing::*` events (used by the
    // composio-side providers, including SlackProvider). Without this,
    // channel-level warn logs from `process_channel` are silent and
    // backfill failures look like silent zeros. Filter respects
    // `RUST_LOG` (e.g. `RUST_LOG=info,openhuman_core=debug`).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .try_init()
        .ok();

    let cli = Cli::parse();

    // Load real on-disk config — same path the full core uses — so
    // `memory_tree.embedding_*`, `llm_extractor_*`, and
    // `llm_summariser_*` settings apply automatically.
    let config = Config::load_or_init()
        .await
        .context("[slack_backfill] Config::load_or_init failed")?;
    std::fs::create_dir_all(&config.workspace_dir).with_context(|| {
        format!(
            "failed to create workspace dir: {}",
            config.workspace_dir.display()
        )
    })?;
    let config = Arc::new(config);

    // Bootstrap the memory global so `SyncState` KV reads/writes work
    // from inside `SlackProvider::sync()`. `init` is idempotent and
    // returns the (possibly pre-existing) client.
    memory::global::init(config.workspace_dir.clone())
        .map_err(|e| anyhow::anyhow!("[slack_backfill] memory::global::init failed: {e}"))?;

    // Register the default Composio providers (gmail, notion, slack).
    // Idempotent — safe even if called twice.
    init_default_providers();

    let provider = get_provider("slack").ok_or_else(|| {
        anyhow::anyhow!("SlackProvider not registered after init_default_providers")
    })?;

    let client = build_composio_client(&config).ok_or_else(|| {
        anyhow::anyhow!(
            "No Composio client — user not signed in (no JWT). \
             Sign in via the desktop app first, then re-run this binary."
        )
    })?;

    if cli.seal_probe {
        use chrono::{Duration, Utc};
        use openhuman_core::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
        use openhuman_core::openhuman::memory::tree::ingest::ingest_chat;

        let connection_id = cli
            .connection_id
            .clone()
            .unwrap_or_else(|| "ca_MSNobId31qJk".to_string());
        let source_id = format!("slack:{connection_id}");
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#seal-probe".into(),
            messages: vec![ChatMessage {
                author: "seal-probe".into(),
                timestamp: Utc::now() - Duration::days(2),
                text: format!(
                    "Seal-cascade probe message at {} — triggers append_leaf \
                     against the existing per-connection source tree's L0 \
                     buffer (already over 10k tokens) so cascade_seals fires \
                     immediately. Used to verify the LlmSummariser→embedder \
                     fix without re-fetching from Composio.",
                    Utc::now().to_rfc3339()
                ),
                source_ref: Some("probe://seal-cascade".into()),
            }],
        };
        log::info!(
            "[slack_backfill] seal-probe: ingesting 1 message under source_id={}",
            source_id
        );
        let result = ingest_chat(
            &config,
            &source_id,
            "",
            vec!["probe".into(), "seal-cascade".into()],
            batch,
        )
        .await
        .context("[slack_backfill] seal-probe ingest_chat failed")?;
        println!(
            "seal-probe done — chunks_written={} chunks_dropped={} chunk_ids={:?}",
            result.chunks_written, result.chunks_dropped, result.chunk_ids
        );
        return Ok(());
    }

    if cli.probe_search {
        // Probe whether the workspace's Slack plan supports
        // `search.messages` (paid plans only). One small query, print
        // raw response, exit. Lets us decide whether to rebuild the
        // provider around SEARCH_MESSAGES (1 paginated call workspace-
        // wide) instead of per-channel `conversations.history` calls.
        let now = chrono::Utc::now();
        let after = (now - chrono::Duration::days(7))
            .format("%Y-%m-%d")
            .to_string();
        let args = serde_json::json!({
            "query": format!("after:{after}"),
            "count": 5,
            "sort": "timestamp",
            "sort_dir": "desc",
        });
        log::info!(
            "[slack_backfill] probing SLACK_SEARCH_MESSAGES with query={}",
            args["query"]
        );
        let resp = client
            .execute_tool("SLACK_SEARCH_MESSAGES", Some(args))
            .await
            .map_err(|e| anyhow::anyhow!("SLACK_SEARCH_MESSAGES failed: {e:#}"))?;
        println!("=== SLACK_SEARCH_MESSAGES probe ===");
        println!("successful: {}", resp.successful);
        println!("error:      {:?}", resp.error);
        println!("cost_usd:   {}", resp.cost_usd);
        println!("data:");
        println!(
            "{}",
            serde_json::to_string_pretty(&resp.data).unwrap_or_default()
        );
        return Ok(());
    }

    let connections = client
        .list_connections()
        .await
        .map_err(|e| anyhow::anyhow!("list_connections failed: {e:#}"))?;

    if cli.use_search {
        let mut slack_conns: Vec<_> = connections
            .connections
            .iter()
            .filter(|c| {
                c.toolkit.eq_ignore_ascii_case("slack")
                    && matches!(c.status.as_str(), "ACTIVE" | "CONNECTED")
            })
            .cloned()
            .collect();
        if let Some(ref wanted) = cli.connection_id {
            slack_conns.retain(|c| &c.id == wanted);
        }
        if slack_conns.is_empty() {
            bail!("no active Slack connection found");
        }
        let started = Instant::now();
        let mut total_buckets = 0usize;
        for conn in &slack_conns {
            let ctx = ProviderContext {
                config: Arc::clone(&config),
                client: client.clone(),
                toolkit: conn.toolkit.clone(),
                connection_id: Some(conn.id.clone()),
            };
            match run_backfill_via_search(&ctx, cli.days).await {
                Ok(outcome) => {
                    total_buckets += outcome.items_ingested;
                    println!(
                        "connection={} buckets={} elapsed_ms={} summary={:?}",
                        conn.id,
                        outcome.items_ingested,
                        outcome.elapsed_ms(),
                        outcome.summary,
                    );
                }
                Err(err) => {
                    eprintln!("connection={} search-backfill failed: {err:#}", conn.id);
                }
            }
        }
        println!(
            "slack-backfill (search) done in {:.1}s — total_buckets={}",
            started.elapsed().as_secs_f64(),
            total_buckets
        );
        return Ok(());
    }

    let mut candidates: Vec<_> = connections
        .connections
        .into_iter()
        .filter(|c| {
            c.toolkit.eq_ignore_ascii_case("slack")
                && matches!(c.status.as_str(), "ACTIVE" | "CONNECTED")
        })
        .collect();

    if let Some(ref wanted) = cli.connection_id {
        candidates.retain(|c| &c.id == wanted);
        if candidates.is_empty() {
            bail!("no active Slack connection found with id={wanted}");
        }
    }

    if candidates.is_empty() {
        bail!(
            "no active Slack connections in Composio. \
             Connect Slack from the desktop app's Integrations screen first."
        );
    }

    log::info!(
        "[slack_backfill] workspace={} connections={} embedder={} extractor={} summariser={}",
        config.workspace_dir.display(),
        candidates.len(),
        component_status(
            &config.memory_tree.embedding_endpoint,
            &config.memory_tree.embedding_model,
        ),
        component_status(
            &config.memory_tree.llm_extractor_endpoint,
            &config.memory_tree.llm_extractor_model,
        ),
        component_status(
            &config.memory_tree.llm_summariser_endpoint,
            &config.memory_tree.llm_summariser_model,
        ),
    );

    let started = Instant::now();
    let mut total_buckets: usize = 0;
    let mut connections_ok: usize = 0;

    for conn in &candidates {
        if cli.reset_state {
            let key = format!("slack:{}", conn.id);
            match memory::global::client_if_ready() {
                Some(mem) => match mem.kv_delete(Some("composio-sync-state"), &key).await {
                    Ok(true) => log::info!(
                        "[slack_backfill] reset SyncState for connection={} (cleared cursors)",
                        conn.id
                    ),
                    Ok(false) => log::info!(
                        "[slack_backfill] no SyncState to reset for connection={}",
                        conn.id
                    ),
                    Err(e) => log::warn!(
                        "[slack_backfill] reset SyncState failed for connection={}: {e:#}",
                        conn.id
                    ),
                },
                None => {
                    log::warn!("[slack_backfill] memory client not ready; skipping --reset-state")
                }
            }
        }
        let ctx = ProviderContext {
            config: Arc::clone(&config),
            client: client.clone(),
            toolkit: conn.toolkit.clone(),
            connection_id: Some(conn.id.clone()),
        };
        match provider.sync(&ctx, SyncReason::Manual).await {
            Ok(outcome) => {
                connections_ok += 1;
                total_buckets += outcome.items_ingested;
                println!(
                    "connection={} buckets_flushed={} elapsed_ms={} summary={:?}",
                    conn.id,
                    outcome.items_ingested,
                    outcome.elapsed_ms(),
                    outcome.summary,
                );
            }
            Err(err) => {
                eprintln!("connection={} sync failed: {err:#}", conn.id);
            }
        }
    }

    println!(
        "slack_backfill done in {:.1}s — connections_ok={}/{} total_buckets_flushed={}",
        started.elapsed().as_secs_f64(),
        connections_ok,
        candidates.len(),
        total_buckets,
    );
    Ok(())
}

fn component_status(endpoint: &Option<String>, model: &Option<String>) -> String {
    match (endpoint.as_deref(), model.as_deref()) {
        (Some(e), Some(m)) if !e.trim().is_empty() && !m.trim().is_empty() => {
            format!("on/{}", m.trim())
        }
        _ => "off".to_string(),
    }
}
