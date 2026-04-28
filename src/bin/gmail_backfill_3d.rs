//! Backfill the last N days of Gmail into the memory-tree content store.
//!
//! Authenticates via Composio (JWT from `<workspace>/auth-profiles.json`),
//! fetches Gmail pages via `GMAIL_FETCH_EMAILS`, converts each thread into an
//! [`EmailThread`], ingests it through `ingest_email` (which writes `.md`
//! files via `content_store` and populates SQLite), then drains the async
//! worker pool until idle.
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
//! ```
//!
//! Set `RUST_LOG=info` (or `debug`) for detailed output.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde_json::{json, Value};

use openhuman_core::openhuman::composio::client::build_composio_client;
use openhuman_core::openhuman::composio::providers::registry::{
    get_provider, init_default_providers,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::tree::canonicalize::email::{EmailMessage, EmailThread};
use openhuman_core::openhuman::memory::tree::content_store::read::verify_chunk_file;
use openhuman_core::openhuman::memory::tree::ingest::{ingest_email, IngestResult};
use openhuman_core::openhuman::memory::tree::jobs::drain_until_idle;
use openhuman_core::openhuman::memory::tree::store::{
    get_chunk_content_pointers, list_chunks, ListChunksQuery,
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

    let source_id = "gmail:backfill";
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

        let chunks_this_page = ingest_page(&config, source_id, &owner, &messages).await?;
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
        let (verified, mismatched, no_pointer, missing_file) = verify_all_chunk_files(&config)?;
        log::info!(
            "[gmail_backfill_3d] integrity done \
             verified={} mismatched={} no_pointer={} missing_file={}",
            verified,
            mismatched,
            no_pointer,
            missing_file,
        );
        if mismatched > 0 || missing_file > 0 {
            anyhow::bail!(
                "Integrity check failed: {} SHA-256 mismatches, {} missing files",
                mismatched,
                missing_file,
            );
        }
    }

    println!(
        "\nBackfill complete. pages={} chunks_written={} cost=~${:.4}",
        total_pages, total_chunks, total_cost,
    );
    Ok(())
}

/// Group a page of raw Gmail messages by (sender, thread_id), convert each
/// thread to an [`EmailThread`], and drive `ingest_email` per thread.
///
/// Returns the total number of chunks written.
async fn ingest_page(
    config: &Config,
    source_id: &str,
    owner: &str,
    page_messages: &[Value],
) -> Result<usize> {
    if page_messages.is_empty() {
        return Ok(0);
    }

    let buckets = bucket_by_thread(page_messages);
    let tags: Vec<String> = vec!["gmail".into(), "ingested".into()];
    let mut total = 0usize;

    for (thread_id, raw_msgs) in &buckets {
        let messages: Vec<EmailMessage> = raw_msgs
            .iter()
            .filter_map(|m| raw_to_email_message(m))
            .collect();
        if messages.is_empty() {
            continue;
        }

        let thread_subject = messages
            .first()
            .map(|m| strip_re_fwd(&m.subject))
            .unwrap_or_else(|| "(no subject)".to_string());

        log::debug!(
            "[gmail_backfill_3d] ingesting thread_id={} messages={}",
            thread_id,
            messages.len(),
        );

        let thread = EmailThread {
            provider: "gmail".to_string(),
            thread_subject,
            messages,
        };

        match ingest_email(config, source_id, owner, tags.clone(), thread).await {
            Ok(IngestResult { chunks_written, .. }) => {
                total += chunks_written;
            }
            Err(e) => {
                log::warn!(
                    "[gmail_backfill_3d] ingest_email failed thread_id={} err={:#}",
                    thread_id,
                    e,
                );
            }
        }
    }

    Ok(total)
}

/// Group raw page messages by thread_id. Within a thread, messages are sorted
/// ascending by date so each thread reads chronologically.
type ThreadBucket<'a> = BTreeMap<String, Vec<&'a Value>>;

fn bucket_by_thread(msgs: &[Value]) -> ThreadBucket<'_> {
    let mut out: ThreadBucket<'_> = BTreeMap::new();
    for m in msgs {
        let thread = m
            .get("threadId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                m.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| format!("solo:{s}"))
            })
            .unwrap_or_else(|| "unknown".to_string());
        out.entry(thread).or_default().push(m);
    }
    for msgs in out.values_mut() {
        msgs.sort_by_key(|m| parse_date(m).map(|d| d.timestamp()).unwrap_or(0));
    }
    out
}

/// Build an [`EmailMessage`] from a raw slim-envelope JSON message.
fn raw_to_email_message(raw: &Value) -> Option<EmailMessage> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let from = raw
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let to = parse_addr_list(raw.get("to"));
    let cc = parse_addr_list(raw.get("cc"));
    let subject = raw
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sent_at = parse_date(raw)?;
    let body = raw
        .get("markdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let source_ref = if id.is_empty() {
        None
    } else {
        Some(format!("gmail://msg/{id}"))
    };

    Some(EmailMessage {
        from,
        to,
        cc,
        subject,
        sent_at,
        body,
        source_ref,
    })
}

/// Parse the `date` field from a raw Gmail message envelope.
fn parse_date(m: &Value) -> Option<DateTime<Utc>> {
    let s = m.get("date").and_then(|v| v.as_str())?;
    s.parse::<DateTime<Utc>>().ok().or_else(|| {
        // Fallback: try RFC 2822 style via humantime or just skip.
        None
    })
}

/// Parse `to` / `cc` fields that may be a JSON array or comma-separated string.
fn parse_addr_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Strip common reply / forward prefixes from a subject line.
fn strip_re_fwd(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lowered = s.to_lowercase();
        if let Some(rest) = lowered
            .strip_prefix("re:")
            .or_else(|| lowered.strip_prefix("fwd:"))
            .or_else(|| lowered.strip_prefix("fw:"))
        {
            s = &s[s.len() - rest.len()..];
            s = s.trim();
        } else {
            break;
        }
    }
    if s.is_empty() {
        "(no subject)".to_string()
    } else {
        s.to_string()
    }
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
