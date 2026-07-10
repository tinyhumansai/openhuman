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
    /// Distinct human participant display names observed in the
    /// transcript (excludes the bot and system/presence lines). Empty
    /// for the local meet-agent flow, which has no transcript to mine.
    /// `#[serde(default)]` keeps older JSONL lines (written before this
    /// field existed) parseable.
    #[serde(default)]
    pub participants: Vec<String>,
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
    file.flush()
        .await
        .map_err(|e| format!("flush {}: {e}", path.display()))?;
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

// ---------------------------------------------------------------------------
// Per-call detail (transcript + summary)
// ---------------------------------------------------------------------------
//
// The recent-calls list (`calls.jsonl`) is deliberately lean so the list
// endpoint stays a cheap whole-file read. The transcript and generated summary
// are heavier and only needed when the user expands one row, so they live in a
// sibling per-call JSON file loaded on demand by `meet_agent_get_call_detail`.

/// One transcript line for a recorded call. `role` is the lowercased speaker
/// role ("participant" / "assistant"); `content` is the line as the backend
/// delivered it (may carry a `[MM:SS] [Name]` prefix).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetCallTranscriptLine {
    pub role: String,
    pub content: String,
}

/// One action item mined from the call. Mirrors the `agent_meetings` summary
/// type but is kept dependency-free here so this store never depends back on
/// `agent_meetings` (which already depends on it — that would be a cycle).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetCallActionItem {
    pub description: String,
    /// `"executable"` or `"advisory"`.
    pub kind: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
}

/// Structured post-call summary persisted alongside the transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetCallSummary {
    pub headline: String,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<MeetCallActionItem>,
}

/// Full detail for a single recorded call: the transcript and the best-effort
/// generated summary. Persisted as one JSON file per call (keyed by
/// `request_id`) so the recent-calls list stays lean and the transcript is
/// only read when a row is expanded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetCallDetail {
    pub request_id: String,
    /// `None` when summarisation failed or timed out — the UI falls back to the
    /// transcript alone. `#[serde(default)]` keeps older files parseable.
    #[serde(default)]
    pub summary: Option<MeetCallSummary>,
    #[serde(default)]
    pub transcript: Vec<MeetCallTranscriptLine>,
}

/// Directory holding per-call detail JSON files — a sibling of `calls.jsonl`.
async fn meet_call_details_dir() -> Result<PathBuf, String> {
    let workspace = Config::load_or_init()
        .await
        .map(|c| c.workspace_dir)
        .map_err(|e| format!("load config: {e}"))?;
    Ok(workspace.join("meet_agent").join("call_details"))
}

/// Map a `request_id` to a filesystem-safe, **injective** file stem.
///
/// ASCII alphanumerics and `-`/`_` pass through unchanged so the common cases —
/// UUID correlation ids and `backend-<ts>` — stay human-readable; every other
/// byte, `%` included, is percent-encoded as `%XX`. Because `%` is itself
/// escaped the mapping is reversible, and therefore collision-free: distinct
/// ids like `a/b`, `a:b`, and `a_b` map to distinct stems (`a%2Fb`, `a%3Ab`,
/// `a_b`) instead of all collapsing onto one file and overwriting each other's
/// detail. It also can never escape the details directory — `/`, `\`, and the
/// `.` in `..` are all encoded.
fn sanitize_stem(request_id: &str) -> String {
    if request_id.is_empty() {
        return "unknown".to_string();
    }
    let mut out = String::with_capacity(request_id.len());
    for &b in request_id.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Persist the detail for one call. Creates the details directory on demand.
/// Best-effort at the call site: callers log and swallow failures since the
/// call is already over and the list row still renders without detail.
pub async fn write_detail(detail: &MeetCallDetail) -> Result<(), String> {
    let dir = meet_call_details_dir().await?;
    write_detail_to(&dir, detail).await
}

async fn write_detail_to(dir: &Path, detail: &MeetCallDetail) -> Result<(), String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.json", sanitize_stem(&detail.request_id)));
    let json = serde_json::to_string(detail).map_err(|e| format!("serialize detail: {e}"))?;
    tokio::fs::write(&path, json.as_bytes())
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Read the detail for one call. Missing file → `Ok(None)` (older calls
/// recorded before this feature, or a call whose detail write failed).
pub async fn read_detail(request_id: &str) -> Result<Option<MeetCallDetail>, String> {
    let dir = meet_call_details_dir().await?;
    read_detail_from(&dir, request_id).await
}

async fn read_detail_from(dir: &Path, request_id: &str) -> Result<Option<MeetCallDetail>, String> {
    let path = dir.join(format!("{}.json", sanitize_stem(request_id)));
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => serde_json::from_str::<MeetCallDetail>(&s)
            .map(Some)
            .map_err(|e| format!("parse {}: {e}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {}: {err}", path.display())),
    }
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
            participants: vec!["Alice".into(), "Bob".into()],
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

    fn sample_detail(request_id: &str) -> MeetCallDetail {
        MeetCallDetail {
            request_id: request_id.to_string(),
            summary: Some(MeetCallSummary {
                headline: "Agreed to ship Friday.".into(),
                key_points: vec!["Ship Friday".into(), "QA owns sign-off".into()],
                action_items: vec![MeetCallActionItem {
                    description: "Send release notes".into(),
                    kind: "executable".into(),
                    tool_name: Some("gmail".into()),
                    assignee: Some("Sam".into()),
                }],
            }),
            transcript: vec![
                MeetCallTranscriptLine {
                    role: "participant".into(),
                    content: "[00:51] [Shanu] your time".into(),
                },
                MeetCallTranscriptLine {
                    role: "assistant".into(),
                    content: "[00:55] [Tiny] On it.".into(),
                },
            ],
        }
    }

    #[tokio::test]
    async fn write_then_read_detail_round_trip() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("call_details");
        let detail = sample_detail("corr-42");
        write_detail_to(&dir, &detail).await.unwrap();
        let got = read_detail_from(&dir, "corr-42").await.unwrap();
        assert_eq!(got.as_ref(), Some(&detail));
    }

    #[tokio::test]
    async fn read_detail_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("call_details");
        // Directory never created → still resolves to None, not an error.
        let got = read_detail_from(&dir, "never-recorded").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn detail_id_with_unsafe_chars_round_trips_via_sanitized_stem() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("call_details");
        // A request_id with path separators must not escape the directory and
        // must read back under the same sanitized stem it was written with.
        let detail = sample_detail("../weird/id:99");
        write_detail_to(&dir, &detail).await.unwrap();
        let got = read_detail_from(&dir, "../weird/id:99").await.unwrap();
        assert_eq!(got, Some(detail));
    }

    #[test]
    fn sanitize_stem_encodes_path_chars_and_never_empty() {
        // Safe chars (alnum, -, _) pass through so UUIDs stay readable.
        assert_eq!(sanitize_stem("abc-DEF_123"), "abc-DEF_123");
        // Path separators / dots are percent-encoded, never bare.
        assert_eq!(sanitize_stem("../a/b.json"), "%2E%2E%2Fa%2Fb%2Ejson");
        assert_eq!(sanitize_stem(""), "unknown");
        assert_eq!(sanitize_stem("///"), "%2F%2F%2F");
    }

    #[test]
    fn sanitize_stem_is_injective_for_ids_that_differ_only_in_punctuation() {
        // The old lossy scheme collapsed all of these onto `a_b`, so the second
        // write clobbered the first. Percent-encoding keeps them distinct.
        let stems = ["a/b", "a:b", "a_b", "a.b"].map(sanitize_stem);
        let unique: std::collections::HashSet<_> = stems.iter().collect();
        assert_eq!(unique.len(), stems.len(), "stems collided: {stems:?}");
    }

    #[tokio::test]
    async fn details_with_punctuation_differing_ids_do_not_overwrite() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("call_details");
        // Two distinct calls whose ids differ only by a separator the old
        // sanitizer flattened — each must keep its own detail file.
        let mut a = sample_detail("call/1");
        a.transcript[0].content = "from call/1".into();
        let mut b = sample_detail("call_1");
        b.transcript[0].content = "from call_1".into();
        write_detail_to(&dir, &a).await.unwrap();
        write_detail_to(&dir, &b).await.unwrap();
        assert_eq!(read_detail_from(&dir, "call/1").await.unwrap(), Some(a));
        assert_eq!(read_detail_from(&dir, "call_1").await.unwrap(), Some(b));
    }
}
