//! Pure, testable single-tick body for the iMessage scanner.
//!
//! `run_scanner` owns the loop, cursor I/O, and AppHandle-dependent path
//! resolution. This module owns "what a tick actually does" so it can be
//! exercised against a real chat.db without a Tauri runtime.

use std::path::{Path, PathBuf};

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

#[allow(dead_code)]
pub(crate) fn chat_db_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct FakeDeps {
        gate: anyhow::Result<Option<Vec<String>>>,
        calls: Arc<Mutex<Vec<(String, String, String)>>>,
        fail_keys: Vec<String>,
    }

    impl FakeDeps {
        fn new(gate: Option<Vec<String>>) -> Self {
            Self {
                gate: Ok(gate),
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_keys: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl TickDeps for FakeDeps {
        async fn fetch_gate(&self) -> anyhow::Result<Option<Vec<String>>> {
            match &self.gate {
                Ok(g) => Ok(g.clone()),
                Err(e) => Err(anyhow::anyhow!("{}", e)),
            }
        }
        async fn ingest_group(
            &self,
            account_id: &str,
            key: &str,
            transcript: String,
        ) -> anyhow::Result<()> {
            if self.fail_keys.iter().any(|k| k == key) {
                anyhow::bail!("forced failure for key={}", key);
            }
            self.calls
                .lock()
                .push((account_id.to_string(), key.to_string(), transcript));
            Ok(())
        }
    }

    fn chat_db() -> Option<PathBuf> {
        super::super::chat_db_path().filter(|p| p.exists())
    }

    #[tokio::test]
    async fn skips_when_gate_disconnected() {
        let deps = FakeDeps::new(None);
        let out = run_single_tick(
            TickInput {
                db_path: PathBuf::from("/nonexistent/chat.db"),
                last_rowid: 0,
                account_id: "test".into(),
            },
            &deps,
        )
        .await
        .unwrap();
        assert!(out.skipped_unconnected);
        assert_eq!(out.groups_attempted, 0);
        assert_eq!(out.new_rowid, 0);
        assert!(deps.calls.lock().is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn run_single_tick_ingests_groups_from_real_chatdb() {
        let Some(db) = chat_db() else {
            eprintln!("chat.db not available — skipping");
            return;
        };
        let deps = FakeDeps::new(Some(vec!["*".into()]));
        let out = run_single_tick(
            TickInput {
                db_path: db,
                last_rowid: 0,
                account_id: "local".into(),
            },
            &deps,
        )
        .await
        .unwrap();
        assert!(!out.skipped_unconnected);
        assert!(
            out.groups_ingested >= 1,
            "expected at least one group ingested from real chat.db, got {:?}",
            out
        );
        assert!(out.new_rowid > 0);
        let calls = deps.calls.lock();
        assert_eq!(calls.len(), out.groups_ingested);
        for (acct, key, transcript) in calls.iter() {
            assert_eq!(acct, "local");
            assert!(key.contains(':'), "key missing YMD: {}", key);
            assert!(!transcript.is_empty());
        }
    }

    #[tokio::test]
    #[ignore]
    async fn run_single_tick_keeps_cursor_on_group_failure() {
        let Some(db) = chat_db() else {
            return;
        };
        // First, sniff one key so we know what to fail.
        let probe = FakeDeps::new(Some(vec!["*".into()]));
        let _ = run_single_tick(
            TickInput {
                db_path: db.clone(),
                last_rowid: 0,
                account_id: "probe".into(),
            },
            &probe,
        )
        .await
        .unwrap();
        let Some(first_key) = probe.calls.lock().first().map(|(_, k, _)| k.clone()) else {
            eprintln!("no groups in chat.db — skipping");
            return;
        };

        let mut deps = FakeDeps::new(Some(vec!["*".into()]));
        deps.fail_keys = vec![first_key];
        let out = run_single_tick(
            TickInput {
                db_path: db,
                last_rowid: 0,
                account_id: "fail".into(),
            },
            &deps,
        )
        .await
        .unwrap();
        assert!(out.had_group_failure);
        assert_eq!(out.new_rowid, 0, "cursor must stay on failure");
    }
}
