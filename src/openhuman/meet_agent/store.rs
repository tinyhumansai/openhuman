//! Persistence for completed meet-agent calls.
//!
//! Append-only JSONL file under the workspace data dir. Each line is
//! one `MeetCallRecord` written when `handle_stop_session` closes a
//! call. The list endpoint reads the tail of the file in reverse so
//! the most recent calls come first — same shape the UI expects.
//!
//! ## Why JSONL (not sqlite)
//!
//! Meet call records are write-rarely, read-rarely, low-cardinality
//! data. A single user closes a few calls per day at most. JSONL is
//! cheap to append (no locking machinery beyond OpenOptions::append),
//! trivial to inspect with `tail`, and survives partial writes — a
//! malformed final line just gets skipped on parse. A sqlite table
//! would add a migration, a connection pool, and a `cargo` build
//! dependency for no real benefit at this volume.
//!
//! ## Bounding
//!
//! `read_recent` caps the in-memory result at `MAX_RECENT_CALLS` so
//! a long-lived install with thousands of calls doesn't allocate an
//! unbounded Vec. The file itself is never truncated here; a future
//! housekeeping job can prune.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::openhuman::config::Config;

/// One closed Meet call. Persisted as a JSONL line.
///
/// Fields use `snake_case` because the RPC layer surfaces them
/// directly (we don't rename when serializing to the frontend), and
/// the JSONL file becomes self-describing for anyone running `tail`
/// on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetCallRecord {
    /// UUID minted by `openhuman.meet_join_call`. Matches the session
    /// key. Stable per call so the UI can dedup if a record is
    /// re-emitted on a rare crash-and-retry path.
    pub request_id: String,
    /// Normalised Meet URL the call joined. Stored so the recent-calls
    /// list can show *which* meeting this was without forcing the
    /// frontend to keep an in-memory map.
    pub meet_url: String,
    /// Bot tile name as typed into Meet's "Your name" input. Useful
    /// when the user runs multiple bot personas.
    pub bot_display_name: String,
    /// Call owner display name (the user who launched the bot).
    /// Snapshotted at start so a later rename in the user profile
    /// doesn't mutate history.
    pub owner_display_name: String,
    /// Wall-clock ms at start_session.
    pub started_at_ms: u64,
    /// Wall-clock ms at stop_session.
    pub ended_at_ms: u64,
    /// Total seconds of inbound (Meet → agent) audio processed.
    pub listened_seconds: f32,
    /// Total seconds of outbound (agent → Meet) audio synthesized.
    pub spoken_seconds: f32,
    /// Completed agent turns during the call.
    pub turn_count: u32,
}

/// Hard cap on the rows returned from `read_recent`. The UI shows ~20
/// rows initially with a "Load more" affordance reserved for later;
/// keeping the API ceiling at 200 means a misconfigured client can't
/// trigger an OOM-shaped read.
pub const MAX_RECENT_CALLS: usize = 200;

/// Resolve the workspace-relative path of the meet-calls JSONL file.
/// Mirrors `threads/ops::workspace_dir` — single source of truth for
/// "where does openhuman keep its per-user data". Created on demand
/// at append time; missing file at read time is treated as "no
/// recorded calls yet" (returns an empty Vec rather than an error).
pub async fn meet_calls_jsonl_path() -> Result<PathBuf, String> {
    let workspace = Config::load_or_init()
        .await
        .map(|c| c.workspace_dir)
        .map_err(|e| format!("load config: {e}"))?;
    Ok(workspace.join("meet_agent").join("calls.jsonl"))
}

/// Append a single record to the JSONL store. Creates parent
/// directories if missing. Each call writes one line + newline so
/// the file remains parsable even when a future writer crashes
/// mid-line (the partial line is skipped on read).
pub async fn append_record(record: &MeetCallRecord) -> Result<(), String> {
    let path = meet_calls_jsonl_path().await?;
    append_record_to(&path, record).await
}

async fn append_record_to(path: &Path, record: &MeetCallRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| format!("serialize: {e}"))?;
    line.push('\n');
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Return the `limit` most recent records (newest first). Missing
/// file → empty Vec. Malformed lines are dropped silently with a
/// debug log so one bad row doesn't poison the whole list. The cap
/// is enforced *after* parsing so future fields don't break older
/// records — readers are tolerant of unknown trailing fields via
/// serde's default behavior.
pub async fn read_recent(limit: usize) -> Result<Vec<MeetCallRecord>, String> {
    let path = meet_calls_jsonl_path().await?;
    read_recent_from(&path, limit).await
}

async fn read_recent_from(path: &Path, limit: usize) -> Result<Vec<MeetCallRecord>, String> {
    let limit = limit.min(MAX_RECENT_CALLS);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("open {}: {err}", path.display())),
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut all: Vec<MeetCallRecord> = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?
    {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MeetCallRecord>(&line) {
            Ok(rec) => all.push(rec),
            Err(err) => {
                log::debug!("[meet-agent-store] skip malformed line err={err}");
            }
        }
    }
    // Newest first. Compare on started_at_ms for stability against
    // future out-of-order writes (e.g. a future async flush race).
    all.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    all.truncate(limit);
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(idx: u64) -> MeetCallRecord {
        MeetCallRecord {
            request_id: format!("req-{idx}"),
            meet_url: "https://meet.google.com/abc-defg-hij".into(),
            bot_display_name: "OpenHuman".into(),
            owner_display_name: "Alice".into(),
            started_at_ms: 1_000_000 + idx * 60_000,
            ended_at_ms: 1_000_000 + idx * 60_000 + 30_000,
            listened_seconds: 12.5,
            spoken_seconds: 4.2,
            turn_count: 3,
        }
    }

    #[tokio::test]
    async fn append_then_read_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("calls.jsonl");
        let a = sample(1);
        let b = sample(2);
        append_record_to(&path, &a).await.unwrap();
        append_record_to(&path, &b).await.unwrap();
        let recent = read_recent_from(&path, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first → req-2 comes before req-1.
        assert_eq!(recent[0].request_id, "req-2");
        assert_eq!(recent[1].request_id, "req-1");
    }

    #[tokio::test]
    async fn read_recent_caps_limit() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("calls.jsonl");
        for i in 0..5 {
            append_record_to(&path, &sample(i)).await.unwrap();
        }
        let recent = read_recent_from(&path, 3).await.unwrap();
        assert_eq!(recent.len(), 3);
        // Top 3 are the most recent (idx 4, 3, 2).
        assert_eq!(recent[0].request_id, "req-4");
        assert_eq!(recent[2].request_id, "req-2");
    }

    #[tokio::test]
    async fn read_recent_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.jsonl");
        let recent = read_recent_from(&path, 10).await.unwrap();
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn malformed_line_is_skipped() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("calls.jsonl");
        // Hand-write a file with one good record + one bad line.
        let good = serde_json::to_string(&sample(1)).unwrap();
        tokio::fs::write(&path, format!("{good}\nnot-json\n"))
            .await
            .unwrap();
        let recent = read_recent_from(&path, 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].request_id, "req-1");
    }

    #[tokio::test]
    async fn zero_limit_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("calls.jsonl");
        append_record_to(&path, &sample(1)).await.unwrap();
        let recent = read_recent_from(&path, 0).await.unwrap();
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn limit_above_cap_is_clamped() {
        // Passing usize::MAX must not allocate Vec::with_capacity(usize::MAX).
        // The clamp lives inside read_recent_from before any allocation.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("calls.jsonl");
        append_record_to(&path, &sample(1)).await.unwrap();
        let recent = read_recent_from(&path, usize::MAX).await.unwrap();
        assert_eq!(recent.len(), 1);
    }
}
