//! Pure, testable single-tick body for the iMessage scanner.
//!
//! `run_scanner` owns the loop, cursor I/O, and AppHandle-dependent path
//! resolution. This module owns "what a tick actually does" so it can be
//! exercised against a real chat.db without a Tauri runtime.

use std::path::PathBuf;

use async_trait::async_trait;

use super::chatdb;
use super::{
    chat_allowed, format_transcript, local_day_bounds_apple_ns, seconds_to_ymd,
    unique_chat_day_keys, MAX_MESSAGES_PER_DAY_REBUILD, MAX_MESSAGES_PER_TICK,
};

pub struct TickInput {
    pub db_path: PathBuf,
    pub last_rowid: i64,
    pub account_id: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickOutcome {
    pub new_rowid: i64,
    pub groups_attempted: usize,
    pub groups_ingested: usize,
    pub skipped_unconnected: bool,
    pub had_group_failure: bool,
}

#[async_trait]
pub trait TickDeps {
    /// Fetch the current iMessage gate:
    /// - `Ok(Some(allowed_contacts))` — connected; empty list = allow all.
    /// - `Ok(None)` — not connected; skip tick.
    /// - `Err(_)` — transport failure; caller retries next tick.
    async fn fetch_gate(&self) -> anyhow::Result<Option<Vec<String>>>;

    async fn ingest_group(
        &self,
        account_id: &str,
        key: &str,
        transcript: String,
    ) -> anyhow::Result<()>;
}

/// One pass of the scanner body: fetch gate, read new rows since
/// `last_rowid`, rebuild each touched (chat, day) from the DB, and hand each
/// transcript to `deps.ingest_group`. Does NOT sleep, persist cursor, or
/// touch AppHandle.
pub async fn run_single_tick<D: TickDeps + ?Sized>(
    input: TickInput,
    deps: &D,
) -> anyhow::Result<TickOutcome> {
    let TickInput {
        db_path,
        last_rowid,
        account_id,
    } = input;

    let allowed_contacts = match deps.fetch_gate().await? {
        Some(a) => a,
        None => {
            return Ok(TickOutcome {
                new_rowid: last_rowid,
                skipped_unconnected: true,
                ..Default::default()
            });
        }
    };

    let messages = chatdb::read_since(&db_path, last_rowid, MAX_MESSAGES_PER_TICK)?;
    if messages.is_empty() {
        return Ok(TickOutcome {
            new_rowid: last_rowid,
            ..Default::default()
        });
    }

    let tick_max_rowid = messages.iter().map(|m| m.rowid).max().unwrap_or(last_rowid);
    let day_keys = unique_chat_day_keys(&messages);

    let mut attempted = 0usize;
    let mut ingested = 0usize;
    let mut had_group_failure = false;

    for (chat_id, anchor_secs) in day_keys {
        if !chat_allowed(&chat_id, &allowed_contacts) {
            continue;
        }
        let (start_ns, end_ns) = local_day_bounds_apple_ns(anchor_secs);
        let full_day = match chatdb::read_chat_day(
            &db_path,
            &chat_id,
            start_ns,
            end_ns,
            MAX_MESSAGES_PER_DAY_REBUILD,
        ) {
            Ok(msgs) => msgs,
            Err(e) => {
                log::warn!("[imessage] full-day read failed chat={} err={}", chat_id, e);
                had_group_failure = true;
                continue;
            }
        };
        if full_day.is_empty() {
            continue;
        }
        let day_ymd = seconds_to_ymd(anchor_secs);
        let key = format!("{}:{}", chat_id, day_ymd);
        let transcript = format_transcript(&full_day);
        attempted += 1;
        match deps.ingest_group(&account_id, &key, transcript).await {
            Ok(()) => ingested += 1,
            Err(e) => {
                log::warn!("[imessage] memory write failed key={} err={}", key, e);
                had_group_failure = true;
            }
        }
    }

    let new_rowid = if had_group_failure {
        last_rowid
    } else {
        tick_max_rowid
    };

    Ok(TickOutcome {
        new_rowid,
        groups_attempted: attempted,
        groups_ingested: ingested,
        skipped_unconnected: false,
        had_group_failure,
    })
}

/// Production deps: hits the real core JSON-RPC surface.
pub struct HttpDeps;

#[async_trait]
impl TickDeps for HttpDeps {
    async fn fetch_gate(&self) -> anyhow::Result<Option<Vec<String>>> {
        super::fetch_imessage_gate().await
    }

    async fn ingest_group(
        &self,
        account_id: &str,
        key: &str,
        transcript: String,
    ) -> anyhow::Result<()> {
        super::ingest_group(account_id, key, transcript).await
    }
}

#[cfg(test)]
#[path = "tick_tests.rs"]
mod tests;
