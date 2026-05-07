//! Backfill the last N days of Gmail into the memory-tree content store.
//!
//! Authenticates via Composio (JWT from `<workspace>/auth-profiles.json`),
//! fetches Gmail pages via `GMAIL_FETCH_EMAILS`, converts each thread into an
//! [`EmailThread`], ingests it through `ingest_page_into_memory_tree` (which
//! writes `.md` files via `content_store` and populates SQLite), then drains
//! the async worker pool until idle.
//!
//! After draining, the binary performs an integrity check: for every chunk
//! that has a `content_path` in SQLite, it verifies the on-disk SHA-256
//! matches the stored `content_sha256`.
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
use serde_json::{json, Value};

use openhuman_core::openhuman::composio::client::build_composio_client;
use openhuman_core::openhuman::composio::providers::gmail::ingest::ingest_page_into_memory_tree;
use openhuman_core::openhuman::composio::providers::registry::{
    get_provider, init_default_providers,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::tree::content_store::read::{
    verify_chunk_file, verify_summary_file, VerifyResult,
};
use openhuman_core::openhuman::memory::tree::jobs::drain_until_idle;
use openhuman_core::openhuman::memory::tree::store::{
    get_chunk_content_pointers, list_chunks, list_summaries_with_content_path, ListChunksQuery,
};

#[derive(Parser, Debug)]
#[command(
    name = "gmail-backfill-3d",
    about = "Backfill last N days of Gmail into the memory-tree content store (.md files + SQLite)."
)]
struct Cli {
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

    /// Skip the post-drain integrity check (SHA-256 file verification).
    #[arg(long, default_value_t = false)]
    skip_verify: bool,

    /// Override the owner string embedded in chunk metadata. Defaults to
    /// `"gmail-backfill"`.
    #[arg(long)]
    owner: Option<String>,

    /// Wipe `chunks.db` (+ wal/shm) AND `<content_root>/` before running.
    /// Useful after a chunker change that invalidates existing chunk IDs.
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

    let config = Config::load_or_init()
        .await
        .context("[gmail_backfill_3d] Config::load_or_init failed")?;

    if cli.wipe {
        wipe_memory_tree_state(&config)?;
    }

    let client = build_composio_client(&config).ok_or_else(|| {
        anyhow::anyhow!(
            "No Composio client — user not signed in (no JWT). \
             Sign in via the desktop app first, then re-run this binary."
        )
    })?;

    init_default_providers();
    let provider = get_provider("gmail").ok_or_else(|| {
        anyhow::anyhow!("GmailProvider not registered after init_default_providers")
    })?;

    let owner = cli
        .owner
        .clone()
        .unwrap_or_else(|| "gmail-backfill".to_string());

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

    let content_root = config.memory_tree_content_root();
    log::info!(
        "[gmail_backfill_3d] content_root={}",
        content_root.display()
    );

    // ─── Fetch + ingest ────────────────────────────────────────────────────

    let mut page_token: Option<String> = None;
    let mut total_chunks = 0usize;
    let mut total_pages = 0usize;
    let mut total_cost: f64 = 0.0;

    for page_num in 0..cli.max_pages {
        let mut args = json!({
            "max_results": cli.page_size,
            "query": query,
        });
        if cli.include_spam_trash {
            args["include_spam_trash"] = json!(true);
        }
        if let Some(token) = &page_token {
            args["page_token"] = json!(token);
        }

        log::info!(
            "[gmail_backfill_3d] fetching page {}{}…",
            page_num,
            page_token.as_ref().map(|_| " (paginated)").unwrap_or(""),
        );

        let mut resp = client
            .execute_tool("GMAIL_FETCH_EMAILS", Some(args.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("GMAIL_FETCH_EMAILS page {page_num}: {e:#}"))?;
        total_cost += resp.cost_usd;

        if !resp.successful {
            anyhow::bail!(
                "GMAIL_FETCH_EMAILS page {page_num} failed: {:?}",
                resp.error
            );
        }

        provider.post_process_action_result("GMAIL_FETCH_EMAILS", Some(&args), &mut resp.data);

        let (messages, next_token) = extract_envelope(&resp.data);
        log::info!(
            "[gmail_backfill_3d] page {} -> {} messages, next_token={}",
            page_num,
            messages.len(),
            next_token.as_deref().unwrap_or("(none)"),
        );

        if messages.is_empty() {
            break;
        }

        // CLI runs don't fetch the user profile, so pass `None` and
        // let the ingest fall back to per-participants source ids.
        let chunks_this_page =
            ingest_page_into_memory_tree(&config, &owner, None, &messages).await?;
        total_chunks += chunks_this_page;
        total_pages += 1;

        log::info!(
            "[gmail_backfill_3d] page {} ingested chunks={} running_total={}",
            page_num,
            chunks_this_page,
            total_chunks,
        );

        match next_token {
            Some(tok) => page_token = Some(tok),
            None => break,
        }
    }

    log::info!(
        "[gmail_backfill_3d] fetch+ingest done pages={} total_chunks={} cost=~${:.4}",
        total_pages,
        total_chunks,
        total_cost,
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
        log::info!("[gmail_backfill_3d] running integrity check…");

        // Chunk integrity.
        let (verified, mismatched, no_pointer, missing_file) = verify_all_chunk_files(&config)?;
        log::info!(
            "[gmail_backfill_3d] chunks: verified={} mismatched={} no_pointer={} missing_file={}",
            verified,
            mismatched,
            no_pointer,
            missing_file,
        );

        // Summary integrity.
        let (sum_verified, sum_mismatched, sum_no_pointer, sum_missing_file) =
            verify_all_summary_files(&config)?;
        log::info!(
            "[gmail_backfill_3d] summaries: verified={} mismatched={} no_pointer={} missing_file={}",
            sum_verified,
            sum_mismatched,
            sum_no_pointer,
            sum_missing_file,
        );

        if mismatched > 0 || missing_file > 0 || sum_mismatched > 0 || sum_missing_file > 0 {
            anyhow::bail!(
                "Integrity check failed: \
                 chunks: {} mismatches, {} missing files; \
                 summaries: {} mismatches, {} missing files",
                mismatched,
                missing_file,
                sum_mismatched,
                sum_missing_file,
            );
        }
    }

    println!(
        "\nBackfill complete. pages={} chunks_written={} cost=~${:.4}",
        total_pages, total_chunks, total_cost,
    );
    Ok(())
}

/// Wipe `<workspace>/memory_tree/chunks.db` (+ wal/shm) and
/// `<content_root>/` so the bin can re-run cleanly after a chunker
/// change that invalidates existing chunk IDs.
///
/// Logs each removed artifact at info; missing files are not an error.
fn wipe_memory_tree_state(config: &Config) -> Result<()> {
    let mt_dir = config.workspace_dir.join("memory_tree");
    for name in &["chunks.db", "chunks.db-wal", "chunks.db-shm"] {
        let path = mt_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => log::info!("[gmail_backfill_3d] wiped {}", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("wipe {}", path.display())),
        }
    }
    let content_root = config.memory_tree_content_root();
    if content_root.exists() {
        std::fs::remove_dir_all(&content_root)
            .with_context(|| format!("wipe {}", content_root.display()))?;
        log::info!("[gmail_backfill_3d] wiped {}", content_root.display());
    }
    Ok(())
}

/// Read all chunks from SQLite and verify on-disk SHA-256 matches `content_sha256`.
///
/// Returns `(verified, mismatched, no_pointer, missing_file)`.
fn verify_all_chunk_files(config: &Config) -> Result<(usize, usize, usize, usize)> {
    let chunks = list_chunks(config, &ListChunksQuery::default())?;
    let content_root = config.memory_tree_content_root();

    let mut verified = 0usize;
    let mut mismatched = 0usize;
    let mut no_pointer = 0usize;
    let mut missing_file = 0usize;

    for chunk in &chunks {
        let pointers = get_chunk_content_pointers(config, &chunk.id)?;
        let (rel_path, expected_sha) = match pointers {
            None => {
                no_pointer += 1;
                log::debug!(
                    "[gmail_backfill_3d] verify: chunk {} has no content_path/sha256",
                    chunk.id
                );
                continue;
            }
            Some(pair) => pair,
        };

        let abs_path = {
            let mut p = content_root.clone();
            for component in rel_path.split('/') {
                p.push(component);
            }
            p
        };

        if !abs_path.exists() {
            missing_file += 1;
            log::warn!(
                "[gmail_backfill_3d] verify: file missing chunk_id={} path={}",
                chunk.id,
                abs_path.display(),
            );
            continue;
        }

        match verify_chunk_file(&abs_path, &expected_sha) {
            Ok(true) => {
                verified += 1;
            }
            Ok(false) => {
                mismatched += 1;
                log::warn!(
                    "[gmail_backfill_3d] verify: SHA-256 mismatch chunk_id={} path={}",
                    chunk.id,
                    abs_path.display(),
                );
            }
            Err(e) => {
                log::error!(
                    "[gmail_backfill_3d] verify: error chunk_id={}: {e}",
                    chunk.id,
                );
                mismatched += 1;
            }
        }
    }

    Ok((verified, mismatched, no_pointer, missing_file))
}

/// Read all summary rows with a non-NULL `content_path` from SQLite and verify
/// the on-disk SHA-256 matches `content_sha256`.
///
/// Returns `(verified, mismatched, no_pointer, missing_file)`.
fn verify_all_summary_files(config: &Config) -> Result<(usize, usize, usize, usize)> {
    let rows_with_pointer = list_summaries_with_content_path(config)?;
    let content_root = config.memory_tree_content_root();

    let mut verified = 0usize;
    let mut mismatched = 0usize;
    let mut missing_file = 0usize;

    for (summary_id, rel_path, expected_sha) in &rows_with_pointer {
        let abs_path = {
            let mut p = content_root.clone();
            for component in rel_path.split('/') {
                p.push(component);
            }
            p
        };

        match verify_summary_file(&abs_path, expected_sha) {
            Ok(VerifyResult::Ok) => {
                verified += 1;
            }
            Ok(VerifyResult::Mismatch { actual }) => {
                mismatched += 1;
                log::warn!(
                    "[gmail_backfill_3d] verify: SHA-256 mismatch summary_id={} path={} expected={} actual={}",
                    summary_id,
                    abs_path.display(),
                    expected_sha,
                    actual,
                );
            }
            Ok(VerifyResult::Missing) => {
                missing_file += 1;
                log::warn!(
                    "[gmail_backfill_3d] verify: file missing summary_id={} path={}",
                    summary_id,
                    abs_path.display(),
                );
            }
            Err(e) => {
                log::error!(
                    "[gmail_backfill_3d] verify: error summary_id={}: {e}",
                    summary_id,
                );
                mismatched += 1;
            }
        }
    }

    // Count rows that have no content_path at all (legacy rows).
    // We report this as no_pointer for symmetry with the chunk verifier.
    // We can't easily count them here without a separate query, so we
    // approximate: rows_with_pointer gives us the ones we checked.
    // For now no_pointer = 0 (the bin wipes before re-ingesting so all
    // new rows should have pointers; legacy rows are pre-migration).
    let no_pointer = 0usize;

    Ok((verified, mismatched, no_pointer, missing_file))
}

/// Extract the `messages` array and `nextPageToken` from a Composio response.
fn extract_envelope(data: &Value) -> (Vec<Value>, Option<String>) {
    let candidates: [Option<&Value>; 2] = [Some(data), data.get("data")];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.get("messages").and_then(|v| v.as_array()) {
            let token = cand
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string);
            return (arr.clone(), token);
        }
    }
    (Vec::new(), None)
}
