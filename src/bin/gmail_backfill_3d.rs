//! Backfill the last N days of Gmail into synchronized memory documents.
//!
//! Authenticates via Composio (JWT from `<workspace>/auth-profiles.json`),
//! fetches and stores Gmail pages through tinycortex, then drains the async
//! document-ingestion worker pool until idle.
//!
//! After draining, the binary verifies that TinyCortex persisted the reported
//! records in the `skill-gmail` namespace.
//!
//! # Prerequisites
//!
//! - Signed-in openhuman session JWT in the same workspace the desktop app
//!   uses (stored at `<workspace>/auth-profiles.json`).
//! - Active Gmail connection on Composio for that user.
//!
//! # Usage
//!
//! ```sh
//! cargo run --bin gmail-backfill-3d
//! cargo run --bin gmail-backfill-3d -- --days 7
//! cargo run --bin gmail-backfill-3d -- --days 14 --page-size 100
//! cargo run --bin gmail-backfill-3d -- --skip-drain
//! cargo run --bin gmail-backfill-3d -- --skip-verify
//! cargo run --bin gmail-backfill-3d -- --wipe
//! ```
//!
//! Set `RUST_LOG=info` (or `debug`) for detailed output.

use anyhow::{Context, Result};
use clap::Parser;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory_queue::drain_until_idle;

#[derive(Parser, Debug)]
#[command(
    name = "gmail-backfill-3d",
    about = "Backfill recent Gmail messages into synchronized memory documents."
)]
struct Cli {
    /// Composio Gmail connection id. Defaults to the configured Gmail source.
    #[arg(long)]
    connection_id: Option<String>,

    /// Lookback window in days. Default 3.
    #[arg(long, default_value_t = 3)]
    days: u32,

    /// Page size per `GMAIL_FETCH_EMAILS` call (1–500).
    #[arg(long, default_value_t = 50)]
    page_size: u32,

    /// Cap on pages we will request. Guards against runaway pagination.
    #[arg(long, default_value_t = 40)]
    max_pages: u32,

    /// Include SPAM and TRASH messages in the fetch.
    #[arg(long, default_value_t = false)]
    include_spam_trash: bool,

    /// Extra Gmail search query AND-ed with the default scope.
    #[arg(long)]
    query: Option<String>,

    /// Skip draining the async worker pool after ingest (useful for quick
    /// smoke-test of file writes only).
    #[arg(long, default_value_t = false)]
    skip_drain: bool,

    /// Skip the post-drain persisted-document count check.
    #[arg(long, default_value_t = false)]
    skip_verify: bool,

    /// Override the owner string embedded in chunk metadata. Defaults to
    /// `"gmail-backfill"`.
    #[arg(long)]
    owner: Option<String>,

    /// Clear synchronized Gmail documents before running.
    #[arg(long, default_value_t = false)]
    wipe: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .try_init()
        .ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .try_init()
        .ok();

    let cli = Cli::parse();
    if cli.days == 0 {
        anyhow::bail!("--days must be >= 1");
    }
    if cli.owner.is_some() {
        log::warn!("[gmail_backfill_3d] --owner is retained for compatibility but ignored");
    }

    let config = Config::load_or_init()
        .await
        .context("[gmail_backfill_3d] Config::load_or_init failed")?;

    let memory = openhuman_core::openhuman::memory::global::init(config.workspace_dir.clone())
        .map_err(anyhow::Error::msg)?;
    if cli.wipe {
        log::info!("[gmail_backfill_3d] clearing skill-gmail documents");
        memory
            .clear_namespace("skill-gmail")
            .await
            .map_err(anyhow::Error::msg)?;
    }
    let documents_before = gmail_document_count(&memory).await?;

    let mut query = format!("in:inbox newer_than:{}d", cli.days);
    if !cli.include_spam_trash {
        query.push_str(" -in:spam -in:trash");
    }
    if let Some(extra) = cli
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        query.push(' ');
        query.push_str(extra);
    }

    log::info!(
        "[gmail_backfill_3d] start days={} page_size={} max_pages={} query={:?}",
        cli.days,
        cli.page_size,
        cli.max_pages,
        query,
    );

    // ─── Fetch + ingest through tinycortex ──────────────────────────────────

    let connection_id = cli
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            config.memory_sources.iter().find_map(|source| {
                (source.kind == openhuman_core::openhuman::memory_sources::SourceKind::Composio
                    && source.toolkit.as_deref() == Some("gmail"))
                .then(|| source.connection_id.clone())
                .flatten()
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no Gmail connection configured; pass --connection-id or add a Gmail memory source"
            )
        })?;
    let outcome = openhuman_core::openhuman::tinycortex::run_gmail_backfill(
        &connection_id,
        &query,
        cli.max_pages as usize,
        cli.page_size as usize,
        &config,
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    log::info!(
        "[gmail_backfill_3d] fetch+ingest done records={} actions={} cost_usd={:.4} note={:?}",
        outcome.records_ingested,
        outcome.actions_called,
        outcome.provider_cost_usd,
        outcome.note,
    );

    // ─── Drain async worker pool ────────────────────────────────────────────

    if cli.skip_drain {
        log::info!("[gmail_backfill_3d] skipping worker pool drain (--skip-drain)");
    } else {
        log::info!("[gmail_backfill_3d] draining async worker pool…");
        drain_until_idle(&config).await?;
        log::info!("[gmail_backfill_3d] worker pool idle");
    }

    // ─── Integrity check ────────────────────────────────────────────────────

    if cli.skip_verify {
        log::info!("[gmail_backfill_3d] skipping integrity check (--skip-verify)");
    } else {
        let documents_after = gmail_document_count(&memory).await?;
        let minimum_expected = documents_before.saturating_add(outcome.records_ingested as usize);
        anyhow::ensure!(
            documents_after >= minimum_expected,
            "persisted Gmail document count {documents_after} is below expected minimum {minimum_expected}"
        );
        log::info!(
            "[gmail_backfill_3d] persisted documents before={} after={} reported={}",
            documents_before,
            documents_after,
            outcome.records_ingested,
        );
    }

    println!(
        "\nBackfill complete. records_ingested={} actions={} cost=~${:.4}",
        outcome.records_ingested, outcome.actions_called, outcome.provider_cost_usd,
    );
    Ok(())
}

async fn gmail_document_count(
    memory: &openhuman_core::openhuman::memory_store::MemoryClientRef,
) -> Result<usize> {
    let value = memory
        .list_documents(Some("skill-gmail"))
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(value
        .get("documents")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len))
}
